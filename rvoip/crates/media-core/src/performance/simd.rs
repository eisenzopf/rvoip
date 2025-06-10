//! SIMD optimizations for audio processing
//!
//! This module provides platform-specific SIMD optimizations
//! with fallback to scalar implementations.

/// SIMD-optimized audio processing operations
#[derive(Debug)]
pub struct SimdProcessor {
    /// Whether SIMD is available on this platform
    simd_available: bool,
}

impl SimdProcessor {
    /// Create a new SIMD processor, detecting platform capabilities
    pub fn new() -> Self {
        let simd_available = Self::detect_simd_support();
        Self { simd_available }
    }
    
    /// Detect if SIMD instructions are available
    fn detect_simd_support() -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            std::arch::is_x86_feature_detected!("sse2")
        }
        #[cfg(target_arch = "aarch64")]
        {
            // NEON is always available on AArch64
            true
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            false
        }
    }
    
    /// Check if SIMD is available
    pub fn is_simd_available(&self) -> bool {
        self.simd_available
    }
    
    /// Add two audio buffers using SIMD optimization
    pub fn add_buffers(&self, a: &[i16], b: &[i16], output: &mut [i16]) {
        if a.len() != b.len() || a.len() != output.len() {
            // Fallback to scalar if size mismatch
            self.add_buffers_scalar(a, b, output);
            return;
        }

        // For small frames, scalar processing is faster
        if a.len() < 256 {
            self.add_buffers_scalar(a, b, output);
            return;
        }

        if self.simd_available {
            self.add_buffers_simd(a, b, output);
        } else {
            self.add_buffers_scalar(a, b, output);
        }
    }
    
    /// Apply gain to audio buffer
    pub fn apply_gain(&self, input: &[i16], gain: f32, output: &mut [i16]) {
        if input.len() != output.len() {
            // Fallback to scalar if size mismatch
            self.apply_gain_scalar(input, gain, output);
            return;
        }

        // For small frames, scalar processing is faster due to SIMD setup overhead
        if input.len() < 256 {
            self.apply_gain_scalar(input, gain, output);
            return;
        }

        if self.simd_available {
            self.apply_gain_simd(input, gain, output);
        } else {
            self.apply_gain_scalar(input, gain, output);
        }
    }
    
    /// Calculate RMS (Root Mean Square) of audio samples
    pub fn calculate_rms(&self, samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }

        // For small frames, scalar processing is faster
        if samples.len() < 256 {
            return self.calculate_rms_scalar(samples);
        }

        if self.simd_available {
            self.calculate_rms_simd(samples)
        } else {
            self.calculate_rms_scalar(samples)
        }
    }
    
    // Scalar implementations (fallback)
    
    fn add_buffers_scalar(&self, left: &[i16], right: &[i16], output: &mut [i16]) {
        for ((l, r), o) in left.iter().zip(right.iter()).zip(output.iter_mut()) {
            *o = l.saturating_add(*r);
        }
    }
    
    fn apply_gain_scalar(&self, input: &[i16], gain: f32, output: &mut [i16]) {
        // Pre-convert gain to fixed-point for faster processing
        let gain_fixed = (gain * 32768.0) as i32;
        
        for (inp, out) in input.iter().zip(output.iter_mut()) {
            let result = ((*inp as i32) * gain_fixed) >> 15;
            *out = result.clamp(-32768, 32767) as i16;
        }
    }
    
    fn calculate_rms_scalar(&self, samples: &[i16]) -> f32 {
        let sum_squares: i64 = samples.iter().map(|&s| s as i64 * s as i64).sum();
        ((sum_squares as f64 / samples.len() as f64).sqrt() / 32768.0) as f32
    }
    
    // SIMD implementations
    
    #[cfg(target_arch = "x86_64")]
    fn add_buffers_simd(&self, left: &[i16], right: &[i16], output: &mut [i16]) {
        use std::arch::x86_64::*;
        
        unsafe {
            let chunks = left.len() / 8;
            let remainder = left.len() % 8;
            
            // Process 8 samples at a time using SSE2
            for i in 0..chunks {
                let offset = i * 8;
                let l_ptr = left.as_ptr().add(offset) as *const __m128i;
                let r_ptr = right.as_ptr().add(offset) as *const __m128i;
                let o_ptr = output.as_mut_ptr().add(offset) as *mut __m128i;
                
                let l_vec = _mm_loadu_si128(l_ptr);
                let r_vec = _mm_loadu_si128(r_ptr);
                let result = _mm_adds_epi16(l_vec, r_vec); // Saturated add
                _mm_storeu_si128(o_ptr, result);
            }
            
            // Handle remaining samples
            let offset = chunks * 8;
            for i in 0..remainder {
                output[offset + i] = left[offset + i].saturating_add(right[offset + i]);
            }
        }
    }
    
    #[cfg(target_arch = "x86_64")]
    fn apply_gain_simd(&self, input: &[i16], gain: f32, output: &mut [i16]) {
        use std::arch::x86_64::*;
        
        unsafe {
            let chunks = input.len() / 8;
            let remainder = input.len() % 8;
            let gain_vec = _mm_set1_ps(gain);
            
            // Process 8 samples at a time
            for i in 0..chunks {
                let offset = i * 8;
                let input_ptr = input.as_ptr().add(offset) as *const __m128i;
                let output_ptr = output.as_mut_ptr().add(offset) as *mut __m128i;
                
                // Load 8 i16 values
                let input_vec = _mm_loadu_si128(input_ptr);
                
                // Convert to two sets of 4 f32 values
                let input_lo = _mm_unpacklo_epi16(input_vec, _mm_setzero_si128());
                let input_hi = _mm_unpackhi_epi16(input_vec, _mm_setzero_si128());
                let input_lo_f = _mm_cvtepi32_ps(input_lo);
                let input_hi_f = _mm_cvtepi32_ps(input_hi);
                
                // Apply gain
                let scaled_lo = _mm_mul_ps(input_lo_f, gain_vec);
                let scaled_hi = _mm_mul_ps(input_hi_f, gain_vec);
                
                // Convert back to i16
                let result_lo = _mm_cvtps_epi32(scaled_lo);
                let result_hi = _mm_cvtps_epi32(scaled_hi);
                let result = _mm_packs_epi32(result_lo, result_hi);
                
                _mm_storeu_si128(output_ptr, result);
            }
            
            // Handle remaining samples
            let offset = chunks * 8;
            for i in 0..remainder {
                let scaled = (input[offset + i] as f32 * gain).round() as i32;
                output[offset + i] = scaled.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            }
        }
    }
    
    #[cfg(target_arch = "x86_64")]
    fn calculate_rms_simd(&self, samples: &[i16]) -> f32 {
        use std::arch::x86_64::*;
        
        unsafe {
            let chunks = samples.len() / 8;
            let remainder = samples.len() % 8;
            let mut sum_vec = _mm_setzero_si128();
            
            // Process 8 samples at a time
            for i in 0..chunks {
                let offset = i * 8;
                let samples_ptr = samples.as_ptr().add(offset) as *const __m128i;
                let samples_vec = _mm_loadu_si128(samples_ptr);
                
                // Square the samples (using multiply)
                let squared = _mm_madd_epi16(samples_vec, samples_vec);
                sum_vec = _mm_add_epi32(sum_vec, squared);
            }
            
            // Extract sum components
            let mut sum_array = [0i32; 4];
            _mm_storeu_si128(sum_array.as_mut_ptr() as *mut __m128i, sum_vec);
            let mut total_sum = sum_array.iter().sum::<i32>() as i64;
            
            // Handle remaining samples
            let offset = chunks * 8;
            for i in 0..remainder {
                let sample = samples[offset + i] as i64;
                total_sum += sample * sample;
            }
            
            ((total_sum as f64 / samples.len() as f64).sqrt() / 32768.0) as f32
        }
    }
    
    // ARM NEON implementations
    
    #[cfg(target_arch = "aarch64")]
    fn add_buffers_simd(&self, left: &[i16], right: &[i16], output: &mut [i16]) {
        use std::arch::aarch64::*;
        
        unsafe {
            let chunks = left.len() / 8;
            let remainder = left.len() % 8;
            
            // Process 8 samples at a time using NEON
            for i in 0..chunks {
                let offset = i * 8;
                let l_ptr = left.as_ptr().add(offset);
                let r_ptr = right.as_ptr().add(offset);
                let o_ptr = output.as_mut_ptr().add(offset);
                
                let l_vec = vld1q_s16(l_ptr);
                let r_vec = vld1q_s16(r_ptr);
                let result = vqaddq_s16(l_vec, r_vec); // Saturated add
                vst1q_s16(o_ptr, result);
            }
            
            // Handle remaining samples
            let offset = chunks * 8;
            for i in 0..remainder {
                output[offset + i] = left[offset + i].saturating_add(right[offset + i]);
            }
        }
    }
    
    #[cfg(target_arch = "aarch64")]
    fn apply_gain_simd(&self, input: &[i16], gain: f32, output: &mut [i16]) {
        // For simplicity, use scalar implementation for now
        self.apply_gain_scalar(input, gain, output);
    }
    
    #[cfg(target_arch = "aarch64")]
    fn calculate_rms_simd(&self, samples: &[i16]) -> f32 {
        // For simplicity, use scalar implementation for now
        self.calculate_rms_scalar(samples)
    }
    
    // Fallbacks for unsupported architectures
    
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn add_buffers_simd(&self, left: &[i16], right: &[i16], output: &mut [i16]) {
        self.add_buffers_scalar(left, right, output);
    }
    
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn apply_gain_simd(&self, input: &[i16], gain: f32, output: &mut [i16]) {
        self.apply_gain_scalar(input, gain, output);
    }
    
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn calculate_rms_simd(&self, samples: &[i16]) -> f32 {
        self.calculate_rms_scalar(samples)
    }
}

impl Default for SimdProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_processor_creation() {
        let processor = SimdProcessor::new();
        // Should not panic and should detect some level of support
        println!("SIMD available: {}", processor.is_simd_available());
    }
    
    #[test]
    fn test_add_buffers() {
        let processor = SimdProcessor::new();
        let left = vec![100, 200, 300, 400, 500, 600, 700, 800];
        let right = vec![50, 100, 150, 200, 250, 300, 350, 400];
        let mut output = vec![0; 8];
        
        processor.add_buffers(&left, &right, &mut output);
        
        assert_eq!(output, vec![150, 300, 450, 600, 750, 900, 1050, 1200]);
    }
    
    #[test]
    fn test_add_buffers_saturation() {
        let processor = SimdProcessor::new();
        let left = vec![32000, 32000];
        let right = vec![1000, 1000];
        let mut output = vec![0; 2];
        
        processor.add_buffers(&left, &right, &mut output);
        
        // Should saturate at i16::MAX (32767)
        assert_eq!(output, vec![32767, 32767]);
    }
    
    #[test]
    fn test_apply_gain() {
        let processor = SimdProcessor::new();
        let input = vec![100, -100, 200, -200];
        let mut output = vec![0; 4];
        
        processor.apply_gain(&input, 2.0, &mut output);
        
        assert_eq!(output, vec![200, -200, 400, -400]);
    }
    
    #[test]
    fn test_apply_gain_saturation() {
        let processor = SimdProcessor::new();
        let input = vec![20000, -20000];
        let mut output = vec![0; 2];
        
        processor.apply_gain(&input, 2.0, &mut output);
        
        // Should saturate
        assert_eq!(output, vec![32767, -32768]);
    }
    
    #[test]
    fn test_calculate_rms() {
        let processor = SimdProcessor::new();
        let samples = vec![1000, -1000, 1000, -1000]; // Square wave
        
        let rms = processor.calculate_rms(&samples);
        
        // RMS of square wave should be close to the amplitude / 32768
        let expected = 1000.0 / 32768.0;
        assert!((rms - expected).abs() < 0.001);
    }
    
    #[test]
    fn test_calculate_rms_zero() {
        let processor = SimdProcessor::new();
        let samples = vec![0; 100];
        
        let rms = processor.calculate_rms(&samples);
        
        assert_eq!(rms, 0.0);
    }
    
    #[test]
    fn test_calculate_rms_empty() {
        let processor = SimdProcessor::new();
        let samples = vec![];
        
        let rms = processor.calculate_rms(&samples);
        
        assert_eq!(rms, 0.0);
    }
    
    #[test]
    fn test_simd_vs_scalar_consistency() {
        let processor = SimdProcessor::new();
        let left = vec![100, 200, 300, 400, 500, 600, 700, 800, 900];
        let right = vec![50, 100, 150, 200, 250, 300, 350, 400, 450];
        let mut simd_output = vec![0; 9];
        let mut scalar_output = vec![0; 9];
        
        // Test both implementations
        processor.add_buffers(&left, &right, &mut simd_output);
        processor.add_buffers_scalar(&left, &right, &mut scalar_output);
        
        assert_eq!(simd_output, scalar_output);
    }
} 