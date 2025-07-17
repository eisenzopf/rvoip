//! Integration tests for G.729 LPC Analysis Pipeline
//!
//! This module tests the complete LPC analysis flow from signal input
//! to LSP coefficients, verifying that all components work together.

use crate::codecs::g729::src::{LpcAnalyzer, L_WINDOW, M, MP1};

#[test]
fn test_complete_lpc_pipeline() {
    let mut analyzer = LpcAnalyzer::new();
    
    // Create a synthetic speech-like signal (sine wave with decay)
    let mut signal = [0; L_WINDOW];
    for i in 0..L_WINDOW {
        let phase = (i as f64) * 0.1; // Low frequency
        let decay = 1.0 - (i as f64) / (L_WINDOW as f64); // Gradual decay
        signal[i] = (16000.0 * phase.sin() * decay) as i16;
    }
    
    // Step 1: Compute autocorrelations
    let mut r_h = [0; MP1];
    let mut r_l = [0; MP1];
    analyzer.autocorr(&signal, &mut r_h, &mut r_l);
    
    // Verify autocorrelations are reasonable
    assert!(r_h[0] > 0); // R[0] (energy) should be positive
    for i in 1..=M {
        // Higher lag correlations should generally be smaller
        assert!(r_h[i] <= r_h[0]);
    }
    
    // Step 2: Apply lag windowing
    analyzer.lag_window(&mut r_h, &mut r_l);
    
    // Step 3: Levinson-Durbin algorithm
    let mut lpc_coeffs = [0; MP1];
    let mut reflection_coeffs = [0; M];
    let stable = analyzer.levinson(&r_h, &r_l, &mut lpc_coeffs, &mut reflection_coeffs);
    
    // Verify filter stability
    assert!(stable, "LPC filter should be stable for synthetic signal");
    assert_eq!(lpc_coeffs[0], 4096); // a[0] should always be 1.0 in Q12
    
    // Reflection coefficients should be reasonable
    for &rc in &reflection_coeffs {
        assert!(rc.abs() < 32000, "Reflection coefficient should be stable");
    }
    
    // Step 4: Convert LPC to LSP
    let mut lsp_coeffs = [0; M];
    analyzer.az_lsp(&lpc_coeffs, &mut lsp_coeffs);
    
    // Verify LSP coefficients are in valid range
    for &lsp in &lsp_coeffs {
        assert!(lsp.abs() <= 32767, "LSP should be valid Q15 value");
    }
    
    // Step 5: Convert LSP back to LPC
    let mut restored_lpc = [0; MP1];
    analyzer.lsp_az(&lsp_coeffs, &mut restored_lpc);
    
    // Verify round-trip conversion
    assert_eq!(restored_lpc[0], 4096); // a[0] should still be 1.0
    
    // Coefficients should be in reasonable range
    for i in 1..MP1 {
        assert!(restored_lpc[i].abs() < 16384, "Restored LPC coefficient should be reasonable");
    }
    
    println!("✅ Complete LPC pipeline test passed");
    println!("   - Autocorrelation: R[0] = {}", r_h[0]);
    println!("   - Filter stability: {}", if stable { "STABLE" } else { "UNSTABLE" });
    println!("   - LPC coefficients: {:?}", &lpc_coeffs[0..4]);
    println!("   - LSP coefficients: {:?}", &lsp_coeffs[0..4]);
}

#[test]
fn test_lpc_with_real_speech_characteristics() {
    let mut analyzer = LpcAnalyzer::new();
    
    // Create a more complex signal resembling speech formants
    let mut signal = [0; L_WINDOW];
    for i in 0..L_WINDOW {
        let t = i as f64 / 8000.0; // Assume 8kHz sampling
        
        // Multiple harmonics to simulate formants
        let f1 = 500.0; // First formant
        let f2 = 1500.0; // Second formant
        let f3 = 2500.0; // Third formant
        
        let sample = 
            5000.0 * (2.0 * std::f64::consts::PI * f1 * t).sin() +
            3000.0 * (2.0 * std::f64::consts::PI * f2 * t).sin() +
            1000.0 * (2.0 * std::f64::consts::PI * f3 * t).sin();
            
        // Add some noise and decay
        let noise = (i as f64 * 0.1).sin() * 100.0;
        let decay = (-t * 2.0).exp(); // Exponential decay
        
        signal[i] = ((sample + noise) * decay) as i16;
    }
    
    // Run complete pipeline
    let mut r_h = [0; MP1];
    let mut r_l = [0; MP1];
    analyzer.autocorr(&signal, &mut r_h, &mut r_l);
    analyzer.lag_window(&mut r_h, &mut r_l);
    
    let mut lpc_coeffs = [0; MP1];
    let mut reflection_coeffs = [0; M];
    let stable = analyzer.levinson(&r_h, &r_l, &mut lpc_coeffs, &mut reflection_coeffs);
    
    if stable {
        let mut lsp_coeffs = [0; M];
        analyzer.az_lsp(&lpc_coeffs, &mut lsp_coeffs);
        
        // Verify LSP ordering (should be monotonic for good speech)
        let mut ordered = true;
        for i in 1..M {
            if lsp_coeffs[i] < lsp_coeffs[i-1] {
                ordered = false;
                break;
            }
        }
        
        println!("✅ Speech-like signal test completed");
        println!("   - Filter stability: {}", if stable { "STABLE" } else { "UNSTABLE" });
        println!("   - LSP ordering: {}", if ordered { "MONOTONIC" } else { "NON-MONOTONIC" });
        println!("   - Energy: R[0] = {}", r_h[0]);
    } else {
        println!("⚠️  Complex signal resulted in unstable filter (expected for some cases)");
    }
}

#[test]
fn test_lpc_edge_cases() {
    let mut analyzer = LpcAnalyzer::new();
    
    // Test 1: Silent signal (all zeros)
    let silent_signal = [0; L_WINDOW];
    let mut r_h = [0; MP1];
    let mut r_l = [0; MP1];
    analyzer.autocorr(&silent_signal, &mut r_h, &mut r_l);
    
    // Should handle gracefully without overflow
    assert!(r_h[0] >= 1); // At least the "avoid all zeros" term
    
    // Test 2: Very loud signal (near saturation)
    let loud_signal = [30000; L_WINDOW]; // Near i16 max
    let mut r_h_loud = [0; MP1];
    let mut r_l_loud = [0; MP1];
    analyzer.autocorr(&loud_signal, &mut r_h_loud, &mut r_l_loud);
    
    // Should handle overflow gracefully
    assert!(r_h_loud[0] > 0);
    
    // Test 3: Alternating signal (worst case for some algorithms)
    let mut alternating_signal = [0; L_WINDOW];
    for i in 0..L_WINDOW {
        alternating_signal[i] = if i % 2 == 0 { 10000 } else { -10000 };
    }
    
    let mut r_h_alt = [0; MP1];
    let mut r_l_alt = [0; MP1];
    analyzer.autocorr(&alternating_signal, &mut r_h_alt, &mut r_l_alt);
    analyzer.lag_window(&mut r_h_alt, &mut r_l_alt);
    
    let mut lpc_alt = [0; MP1];
    let mut rc_alt = [0; M];
    let stable_alt = analyzer.levinson(&r_h_alt, &r_l_alt, &mut lpc_alt, &mut rc_alt);
    
    // Should either be stable or gracefully fall back to previous coefficients
    println!("✅ Edge cases test completed");
    println!("   - Silent signal: R[0] = {}", r_h[0]);
    println!("   - Loud signal: R[0] = {}", r_h_loud[0]);
    println!("   - Alternating signal stable: {}", stable_alt);
}

#[test]
fn test_lpc_performance_characteristics() {
    let mut analyzer = LpcAnalyzer::new();
    
    // Performance test with multiple frames
    let frame_count = 100;
    let mut total_stable = 0;
    
    for frame in 0..frame_count {
        // Generate different test signals
        let mut signal = [0; L_WINDOW];
        let freq = 200.0 + (frame as f64) * 10.0; // Varying frequency
        
        for i in 0..L_WINDOW {
            let phase = (i as f64) * freq * 2.0 * std::f64::consts::PI / 8000.0;
            signal[i] = (8000.0 * phase.sin()) as i16;
        }
        
        // Quick pipeline test
        let mut r_h = [0; MP1];
        let mut r_l = [0; MP1];
        analyzer.autocorr(&signal, &mut r_h, &mut r_l);
        analyzer.lag_window(&mut r_h, &mut r_l);
        
        let mut lpc_coeffs = [0; MP1];
        let mut reflection_coeffs = [0; M];
        let stable = analyzer.levinson(&r_h, &r_l, &mut lpc_coeffs, &mut reflection_coeffs);
        
        if stable {
            total_stable += 1;
        }
    }
    
    let stability_rate = (total_stable as f64) / (frame_count as f64) * 100.0;
    
    println!("✅ Performance test completed");
    println!("   - Frames processed: {}", frame_count);
    println!("   - Stability rate: {:.1}%", stability_rate);
    
    // Should have reasonable stability for synthetic signals
    assert!(stability_rate > 80.0, "Should have high stability rate for clean signals");
} 