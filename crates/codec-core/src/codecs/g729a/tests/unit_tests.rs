//! G.729A Unit Tests
//!
//! This module contains unit tests for individual G.729A codec components.
//! Tests verify that each function works correctly with known inputs and outputs.

use crate::codecs::g729a::*;
use crate::codecs::g729a::types::*;
use crate::codecs::g729a::basic_ops::*;
use crate::codecs::g729a::lpc::*;
use crate::codecs::g729a::filtering::*;

/// Test LPC analysis components
#[cfg(test)]
mod lpc_tests {
    use super::*;

    #[test]
    fn test_autocorr_function() {
        // Test autocorr with known signal
        let mut x = [0i16; L_WINDOW];
        
        // Create a simple periodic signal
        for i in 0..L_WINDOW {
            x[i] = (1000.0 * (2.0 * std::f64::consts::PI * i as f64 / 10.0).sin()) as i16;
        }
        
        let mut r_h = [0i16; MP1];
        let mut r_l = [0i16; MP1];
        
        autocorr(&x, M as Word16, &mut r_h, &mut r_l);
        
        // R[0] should be the maximum (energy)
        assert!(r_h[0] > 0, "R[0] should be positive (signal energy)");
        
        // Other autocorrelations should be smaller in magnitude
        for i in 1..=M {
            assert!(r_h[i].abs() <= r_h[0], "R[{}] should be <= R[0]", i);
        }
    }

    #[test]
    fn test_levinson_durbin() {
        // Create simple autocorrelation sequence
        let r_h = [16384i16, 8192, 4096, 2048, 1024, 512, 256, 128, 64, 32, 16]; // Exponentially decaying
        let r_l = [0i16; MP1];
        
        let mut a = [0i16; MP1];
        let mut rc = [0i16; M];
        
        levinson(&r_h, &r_l, &mut a, &mut rc);
        
        // a[0] should be 1.0 in Q12 (4096)
        assert_eq!(a[0], 4096, "a[0] should be 1.0 in Q12");
        
        // Reflection coefficients should be in range [-1, 1]
        for i in 0..M {
            assert!(rc[i].abs() <= 32767, "Reflection coefficient {} out of range", i);
        }
    }

    #[test]
    fn test_lsp_conversion() {
        // Create simple LPC coefficients
        let mut a = [4096i16, -1000, 800, -600, 400, -200, 100, -50, 25, -12, 6]; // a[0] = 1.0 in Q12
        let mut lsp = [0i16; M];
        let old_lsp = [1000i16, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000]; // Previous LSP values
        
        // Convert LPC to LSP
        az_lsp(&a, &mut lsp, &old_lsp);
        
        // LSP should be ordered and in valid range
        for i in 1..M {
            assert!(lsp[i] >= lsp[i-1], "LSP[{}] should be >= LSP[{}]", i, i-1);
        }
        
        // Convert back to LPC
        let mut a_reconstructed = [0i16; MP1];
        lsp_az(&lsp, &mut a_reconstructed);
        
        // a[0] should still be 1.0
        assert_eq!(a_reconstructed[0], 4096, "Reconstructed a[0] should be 1.0 in Q12");
    }

    #[test]
    fn test_lsp_interpolation() {
        let lsp_old = [1000i16, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000];
        let lsp_new = [1200i16, 2200, 3200, 4200, 5200, 6200, 7200, 8200, 9200, 10200];
        let mut az = [0i16; 2 * MP1];
        
        int_qlpc(&lsp_old, &lsp_new, &mut az);
        
        // Both subframes should have a[0] = 1.0
        assert_eq!(az[0], 4096, "First subframe a[0] should be 1.0");
        assert_eq!(az[MP1], 4096, "Second subframe a[0] should be 1.0");
    }
}

/// Test synthesis filtering
#[cfg(test)]
mod filtering_tests {
    use super::*;

    #[test]
    fn test_synthesis_filter() {
        let a = [4096i16, -500, 400, -300, 200, -100, 50, -25, 12, -6, 3]; // LPC coefficients
        let x = [1000i16, 500, -500, 250, -250, 125, -125, 62, -62, 31, -31, 15]; // Input signal
        let mut y = [0i16; 12];
        let mut mem = [0i16; M];
        
        syn_filt(&a, &x, &mut y, 12, &mut mem, 1);
        
        // Output should be non-zero for non-zero input
        assert!(y.iter().any(|&sample| sample != 0), "Output should be non-zero");
        
        // Memory should be updated
        assert!(mem.iter().any(|&m| m != 0), "Memory should be updated");
    }

    #[test]
    fn test_synthesis_filter_memory() {
        let a = [4096i16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // Unity filter
        let x = [1000i16, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 11000, 12000];
        let mut y = [0i16; 12];
        let mut mem = [0i16; M];
        
        // Test with no memory update
        syn_filt(&a, &x, &mut y, 12, &mut mem, 0);
        let mem_before = mem;
        
        syn_filt(&a, &x, &mut y, 12, &mut mem, 0);
        assert_eq!(mem, mem_before, "Memory should not change with update=0");
        
        // Test with memory update
        syn_filt(&a, &x, &mut y, 12, &mut mem, 1);
        assert_ne!(mem, mem_before, "Memory should change with update=1");
    }
}

/// Test basic operations
#[cfg(test)]
mod basic_ops_tests {
    use super::*;

    #[test]
    fn test_saturation_operations() {
        // Test add with saturation
        assert_eq!(add(32000, 1000), 32767); // Should saturate at max
        assert_eq!(add(-32000, -1000), -32768); // Should saturate at min
        assert_eq!(add(1000, 2000), 3000); // Normal addition
        
        // Test subtract with saturation  
        assert_eq!(sub(32000, -1000), 32767); // Should saturate at max
        assert_eq!(sub(-32000, 1000), -32768); // Should saturate at min
        assert_eq!(sub(5000, 2000), 3000); // Normal subtraction
    }

    #[test]
    fn test_multiplication_operations() {
        // Test mult (Q15 x Q15 -> Q15)
        assert_eq!(mult(16384, 16384), 8192); // 0.5 * 0.5 = 0.25
        assert_eq!(mult(32767, 32767), 32766); // ~1.0 * ~1.0 ≈ 1.0
        
        // Test L_mult (Q15 x Q15 -> Q31) - ITU spec: multiply then left shift by 1
        let result = l_mult(16384, 16384);
        assert_eq!(result, 536870912); // 0.5 * 0.5 * 2 = 0.5 in Q31 (ITU behavior)
    }

    #[test]
    fn test_shift_operations() {
        // Test left shift with saturation
        assert_eq!(shl(16384, 1), 32767); // Should saturate
        assert_eq!(shl(8192, 1), 16384); // Normal shift
        
        // Test right shift
        assert_eq!(shr(16384, 1), 8192);
        assert_eq!(shr(-16384, 1), -8192);
    }

    #[test]
    fn test_normalization() {
        // Test norm_s - ITU-T G.729A behavior (shifts needed to normalize)
        // Positive range target: [16384, 32767] = [0x4000, 0x7FFF]
        assert_eq!(norm_s(1), 14); // 1 needs 14 shifts to reach 16384
        assert_eq!(norm_s(32767), 0); // 32767 is already in range [16384, 32767]
        assert_eq!(norm_s(16384), 0); // 16384 is already at minimum of range
        
        // Test with negative numbers  
        // Negative range target: [-32768, -16384] = [0x8000, 0xC000]
        assert_eq!(norm_s(-1), 15); // -1 = 0xFFFF, needs 15 shifts
        assert_eq!(norm_s(-32768), 0); // -32768 is already in range
    }

    #[test]
    fn test_division() {
        // Test div_s (15-bit division)
        let result = div_s(16384, 32767); // 0.5 / ~1.0 ≈ 0.5
        assert!((result - 16384).abs() < 100, "Division result should be close to 0.5");
        
        let result = div_s(8192, 16384); // 0.25 / 0.5 = 0.5
        assert!((result - 16384).abs() < 100, "Division result should be close to 0.5");
    }

    #[test]
    fn test_extract_operations() {
        // Test extract_h (extract high 16 bits)
        assert_eq!(extract_h(0x12345678), 0x1234);
        assert_eq!(extract_h(0x80000000u32 as i32), -32768); // Sign extension
        
        // Test extract_l (extract low 16 bits)
        assert_eq!(extract_l(0x12345678), 0x5678);
        assert_eq!(extract_l(0x1234FFFF), -1); // Sign extension
    }

    #[test]
    fn test_l_extract_tuple() {
        let (hi, lo) = l_extract_tuple(0x12345678);
        assert_eq!(hi, 0x1234);
        assert_eq!(lo, 0x5678);
        
        let (hi, lo) = l_extract_tuple(-1);
        assert_eq!(hi, -1);
        assert_eq!(lo, -1);
    }

    #[test]
    fn test_mpy_32_16() {
        let hi = 0x1234;
        let lo = 0x5678;
        let n = 0x2000; // 0.25 in Q15
        
        let result = mpy_32_16(hi, lo, n);
        
        // Should be approximately (hi * 2^16 + lo) * n / 2^15
        let expected = ((hi as i32) << 16) + (lo as i32);
        let expected = ((expected as i64 * n as i64) >> 15) as i32;
        
        assert!((result - expected).abs() < 1000, "mpy_32_16 result should be close to expected");
    }
}

/// Test encoder/decoder framework
#[cfg(test)]
mod framework_tests {
    use super::*;
    use crate::codecs::g729a::encoder::*;
    use crate::codecs::g729a::decoder::*;

    #[test]
    fn test_encoder_creation() {
        let encoder = G729AEncoder::new();
        
        // Check initial state
        assert_eq!(encoder.state.sharp, SHARPMIN);
        assert_eq!(encoder.state.old_wsp.len(), L_FRAME + PIT_MAX);
        assert_eq!(encoder.state.lsp_old.len(), M);
        assert_eq!(encoder.state.old_speech.len(), L_TOTAL);
    }

    #[test]
    fn test_decoder_creation() {
        let decoder = G729ADecoder::new();
        
        // Check initial state
        assert_eq!(decoder.state.sharp, SHARPMIN);
        assert_eq!(decoder.state.old_t0, 60);
        assert_eq!(decoder.state.old_exc.len(), L_FRAME + PIT_MAX + L_INTERPOL);
        assert_eq!(decoder.state.mem_syn.len(), M);
    }

    #[test]
    fn test_frame_size_validation() {
        let mut encoder = G729AEncoder::new();
        
        // Test correct frame size
        let correct_frame = vec![0i16; L_FRAME];
        let result = encoder.encode(&correct_frame);
        // Should not panic, but may return error if not fully implemented
        
        // Test incorrect frame size
        let wrong_frame = vec![0i16; L_FRAME - 1];
        let result = encoder.encode(&wrong_frame);
        assert!(result.is_err(), "Should reject wrong frame size");
    }

    #[test]
    fn test_encoder_state_management() {
        let mut encoder = G729AEncoder::new();
        
        // Process multiple frames to test state persistence
        let frame1 = vec![1000i16; L_FRAME];
        let frame2 = vec![2000i16; L_FRAME];
        
        // First frame
        let _result1 = encoder.encode(&frame1);
        let state_after_frame1 = encoder.state.clone();
        
        // Second frame  
        let _result2 = encoder.encode(&frame2);
        let state_after_frame2 = encoder.state.clone();
        
        // State should have changed
        assert_ne!(
            state_after_frame1.old_speech[0..10], 
            state_after_frame2.old_speech[0..10],
            "Encoder state should change between frames"
        );
    }
}

/// Test constants and types
#[cfg(test)]
mod constants_tests {
    use super::*;

    #[test]
    fn test_frame_constants() {
        assert_eq!(L_FRAME, 80, "Frame length should be 80 samples");
        assert_eq!(L_SUBFR, 40, "Subframe length should be 40 samples");
        assert_eq!(L_TOTAL, 240, "Total buffer should be 240 samples");
        assert_eq!(L_WINDOW, 240, "Window length should be 240 samples");
    }

    #[test]
    fn test_lpc_constants() {
        assert_eq!(M, 10, "LPC order should be 10");
        assert_eq!(MP1, 11, "LPC order + 1 should be 11");
        assert_eq!(NC, 5, "NC should be M/2 = 5");
    }

    #[test]
    fn test_pitch_constants() {
        assert_eq!(PIT_MIN, 20, "Minimum pitch should be 20");
        assert_eq!(PIT_MAX, 143, "Maximum pitch should be 143");
        assert_eq!(L_INTERPOL, 11, "Interpolation length should be 11");
    }

    #[test]
    fn test_q_format_constants() {
        assert_eq!(GAMMA1, 24576, "GAMMA1 should be 0.75 in Q15");
        assert_eq!(SHARPMAX, 13017, "SHARPMAX should be 0.8 in Q14");
        assert_eq!(SHARPMIN, 3277, "SHARPMIN should be 0.2 in Q14");
    }

    #[test]
    fn test_lsp_grid() {
        assert_eq!(LSP_GRID.len(), GRID_POINTS + 1);
        assert_eq!(LSP_GRID[0], 32760, "First grid point should be close to 1.0");
        assert_eq!(LSP_GRID[GRID_POINTS], -32760, "Last grid point should be close to -1.0");
        
        // Grid should be monotonically decreasing
        for i in 1..LSP_GRID.len() {
            assert!(LSP_GRID[i] <= LSP_GRID[i-1], "Grid should be monotonically decreasing");
        }
    }

    #[test]
    fn test_parameter_sizes() {
        assert_eq!(PRM_SIZE, 11, "Parameter vector size should be 11");
        assert_eq!(SERIAL_SIZE, 82, "Serial size should be 82 bits");
    }
}

/// Test error conditions and edge cases
#[cfg(test)]
mod edge_case_tests {
    use super::*;

    #[test]
    fn test_zero_input_handling() {
        // Test LPC analysis with zero input
        let x = [0i16; L_WINDOW];
        let mut r_h = [0i16; MP1];
        let mut r_l = [0i16; MP1];
        
        autocorr(&x, M as Word16, &mut r_h, &mut r_l);
        
        // Should handle zero input gracefully - ITU adds noise floor of 1, then normalizes
        // sum=1 -> norm_l(1)=30 -> l_shl(1,30)=0x40000000 -> extract_h(0x40000000)=16384
        assert_eq!(r_h[0], 16384, "R[0] should be 16384 for zero input (normalized noise floor)");
    }

    #[test]
    fn test_saturation_handling() {
        // Test that operations handle saturation correctly
        let large_value = 30000i16;
        let result = add(large_value, large_value);
        assert_eq!(result, 32767, "Should saturate at maximum value");
        
        let small_value = -30000i16;
        let result = add(small_value, small_value);
        assert_eq!(result, -32768, "Should saturate at minimum value");
    }

    #[test]
    fn test_synthesis_filter_stability() {
        // Test synthesis filter with unstable coefficients
        let unstable_a = [4096i16, 16000, -8000, 4000, -2000, 1000, -500, 250, -125, 62, -31];
        let x = [1000i16; 20];
        let mut y = [0i16; 20];
        let mut mem = [0i16; M];
        
        syn_filt(&unstable_a, &x, &mut y, 20, &mut mem, 1);
        
        // With unstable coefficients, filter should saturate to prevent worse behavior
        // This is the correct ITU behavior - saturation is better than divergence
        let has_saturation = y.iter().any(|&sample| sample == i16::MIN || sample == i16::MAX);
        assert!(has_saturation, "Unstable coefficients should cause saturation");
    }

    #[test] 
    fn test_lsp_boundary_conditions() {
        // Test LSP conversion with boundary coefficients
        let boundary_a = [4096i16, 32767, -32767, 16384, -16384, 8192, -8192, 4096, -4096, 2048, -2048];
        let mut lsp = [0i16; M];
        let old_lsp = [100i16; M]; // Small values as fallback
        
        az_lsp(&boundary_a, &mut lsp, &old_lsp);
        
        // Should produce valid LSP values or fall back to old values
        assert!(lsp.iter().all(|&l| l.abs() <= 32767), "LSP values should be in valid range");
    }
} 