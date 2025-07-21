//! Debug test for Levinson-Durbin algorithm

use crate::codecs::g729a::types::{Q15, Q31};
use crate::codecs::g729a::spectral::{LinearPredictor, LSPDecoder};
use crate::codecs::g729a::constants::LP_ORDER;
use std::time::{Duration, Instant};
use std::fs::File;
use std::io::Read;

#[test]
fn test_find_reference_frame() {
    println!("=== FIND WHICH FRAME MATCHES REFERENCE LSP INDICES ===");
    
    // Read the ITU-T test input
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    println!("ALGTHM.IN file size: {} bytes", buffer.len());
    println!("Total frames: {}", buffer.len() / 160);
    
    // ITU-T reference LSP indices from ALGTHM.BIT frame 0
    let reference_indices = [105u8, 17u8, 0u8, 0u8];
    let mut decoder = LSPDecoder::new();
    let reference_lsp = decoder.decode(&reference_indices);
    
    println!("ITU-T Reference LSP frequencies: {:?}", 
        reference_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Convert reference LSP to LP coefficients
    use crate::codecs::g729a::spectral::LSPConverter;
    let converter = LSPConverter::new();
    let reference_lp = converter.lsp_to_lp(&reference_lsp);
    
    println!("ITU-T Reference LP coefficients: {:?}", 
        reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Test multiple frames to see which one is closest  
    let predictor = LinearPredictor::new();
    let mut best_match_frame = 0;
    let mut best_match_distance = i64::MAX;
    
    for frame_idx in 0..4.min(buffer.len() / 160) {
        println!("\n--- Testing Frame {} ---", frame_idx);
        
        let frame_start = frame_idx * 160;
        let mut input_frame = Vec::new();
        
        // Read 80 samples for this frame
        for i in 0..80 {
            let sample_idx = frame_start + i * 2;
            if sample_idx + 1 < buffer.len() {
                let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
                input_frame.push(Q15(sample));
            }
        }
        
        // Check if frame has reasonable speech energy
        let energy: i64 = input_frame.iter().map(|&x| (x.0 as i64) * (x.0 as i64)).sum();
        println!("Frame {} energy: {}", frame_idx, energy);
        
        if energy < 1000 {
            println!("Skipping low-energy frame {}", frame_idx);
            continue;
        }
        
        // Apply windowing - NOTE: Real G.729A needs 240-sample window with history!
        // For now, just use 80-sample frame for comparison
        use crate::codecs::g729a::tables::window_tables::get_hamming_window;
        let hamming_window = get_hamming_window();
        
        let mut windowed = Vec::new();
        for i in 0..80 {
            let windowed_sample = crate::codecs::g729a::math::fixed_point::mult(
                input_frame[i].0, hamming_window[i].0
            );
            windowed.push(Q15(windowed_sample));
        }
        
        // Compute autocorrelation and LP analysis
        use crate::codecs::g729a::math::dsp_operations::{autocorrelation, apply_lag_window};
        use crate::codecs::g729a::tables::window_tables::get_lag_window;
        
        let autocorr = autocorrelation(&windowed, LP_ORDER);
        let lag_window = get_lag_window();
        let mut windowed_autocorr = autocorr.clone();
        apply_lag_window(&mut windowed_autocorr, &lag_window);
        
        let (lp_coeffs, _) = predictor.levinson_durbin(&windowed_autocorr);
        
        // Calculate distance from reference LP coefficients
        let mut distance = 0i64;
        for i in 0..LP_ORDER {
            let diff = lp_coeffs[i].0 as i64 - reference_lp.values[i].0 as i64;
            distance += diff * diff;
        }
        
        println!("LP coeffs: {:?}", lp_coeffs.iter().map(|x| x.0).collect::<Vec<_>>());
        println!("Distance from reference: {}", distance);
        
        if distance < best_match_distance {
            best_match_distance = distance;
            best_match_frame = frame_idx;
        }
    }
    
    println!("\n=== RESULTS ===");
    println!("Best matching frame: {} (distance: {})", best_match_frame, best_match_distance);
    println!("ITU-T Reference LP: {:?}", reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    if best_match_distance > 1000000 {
        println!("\n❌ MAJOR ISSUE: No frame produces coefficients close to ITU-T reference!");
        println!("This suggests:");
        println!("1. We need the full 240-sample window (history + current + lookahead)");
        println!("2. The reference might be for a different input sequence");
        println!("3. There might be different preprocessing in G.729A vs our implementation");
    }
}

#[test]
fn test_debug_autocorrelation_step_by_step() {
    println!("=== DEBUG AUTOCORRELATION STEP BY STEP ===");
    
    // Read the ITU-T test input
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    // Debug: Check file size and look for non-zero data
    println!("ALGTHM.IN file size: {} bytes", buffer.len());
    println!("Expected frames: {}", buffer.len() / 160); // 80 samples * 2 bytes per sample
    
    // Check first few frames for non-zero data
    for frame_idx in 0..4 {
        let frame_start = frame_idx * 160; // 80 samples * 2 bytes
        if frame_start + 160 <= buffer.len() {
            let mut non_zero_count = 0;
            let mut max_abs = 0i16;
            let mut first_nonzero_pos = None;
            
            for i in 0..80 {
                let sample_idx = frame_start + i * 2;
                if sample_idx + 1 < buffer.len() {
                    let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
                    if sample != 0 {
                        non_zero_count += 1;
                        max_abs = max_abs.max(sample.abs());
                        if first_nonzero_pos.is_none() {
                            first_nonzero_pos = Some(i);
                        }
                    }
                }
            }
            
            println!("Frame {}: {} non-zero samples, max_abs={}, first_nonzero_at={:?}", 
                frame_idx, non_zero_count, max_abs, first_nonzero_pos);
        }
    }
    
    // Use frame 1 if frame 0 is silent (common in speech files)
    let frame_to_analyze = if buffer.len() >= 320 { 1 } else { 0 };
    println!("\n=== ANALYZING FRAME {} ===", frame_to_analyze);
    
    // Parse specified frame (80 samples, 16-bit little-endian)
    let frame_start = frame_to_analyze * 160;
    let mut input_frame = Vec::new();
    for i in 0..80 {
        let sample_idx = frame_start + i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            input_frame.push(Q15(sample));
        }
    }
    
    println!("Input frame (first 10 samples): {:?}", 
        input_frame[..10].iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Find first non-zero sample
    let first_nonzero = input_frame.iter().position(|&x| x.0 != 0);
    if let Some(pos) = first_nonzero {
        println!("First non-zero sample at position {}: {}", pos, input_frame[pos].0);
        let end_pos = (pos + 10).min(input_frame.len());
        println!("Samples around position {}: {:?}", pos, 
            input_frame[pos..end_pos].iter().map(|x| x.0).collect::<Vec<_>>());
    } else {
        println!("Frame is silent (all zeros)");
        return; // Skip analysis of silent frame
    }
    
    // Apply Hamming window
    use crate::codecs::g729a::tables::window_tables::get_hamming_window;
    let hamming_window = get_hamming_window();
    
    let mut windowed = Vec::new();
    for i in 0..80 {
        let windowed_sample = crate::codecs::g729a::math::fixed_point::mult(
            input_frame[i].0, hamming_window[i].0
        );
        windowed.push(Q15(windowed_sample));
    }
    
    println!("Windowed frame (first 10 samples): {:?}", 
        windowed[..10].iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Find first non-zero windowed sample
    let first_nonzero_windowed = windowed.iter().position(|&x| x.0 != 0);
    if let Some(pos) = first_nonzero_windowed {
        println!("First non-zero windowed sample at position {}: {}", pos, windowed[pos].0);
        let end_pos = (pos + 10).min(windowed.len());
        println!("Windowed samples around position {}: {:?}", pos, 
            windowed[pos..end_pos].iter().map(|x| x.0).collect::<Vec<_>>());
    }
    
    // Compute autocorrelation with detailed debug
    use crate::codecs::g729a::math::dsp_operations::{autocorrelation};
    
    println!("\n=== AUTOCORRELATION COMPUTATION ===");
    let autocorr = autocorrelation(&windowed, LP_ORDER);
    
    println!("Raw autocorrelation R[0] to R[10]:");
    for i in 0..=LP_ORDER {
        println!("  R[{}] = {}", i, autocorr[i].0);
    }
    
    // Apply lag window  
    use crate::codecs::g729a::tables::window_tables::get_lag_window;
    use crate::codecs::g729a::math::dsp_operations::apply_lag_window;
    let lag_window = get_lag_window();
    
    let mut windowed_autocorr = autocorr.clone();
    apply_lag_window(&mut windowed_autocorr, &lag_window);
    
    println!("\nLag-windowed autocorrelation R[0] to R[10]:");
    for i in 0..=LP_ORDER {
        println!("  R[{}] = {}", i, windowed_autocorr[i].0);
    }
    
    // Run Levinson-Durbin  
    let predictor = LinearPredictor::new();
    println!("\n=== LEVINSON-DURBIN RECURSION ===");
    let (lp_coeffs, reflection_coeffs) = predictor.levinson_durbin(&windowed_autocorr);
    
    println!("Final LP coefficients:");
    for i in 0..LP_ORDER {
        println!("  a[{}] = {}", i+1, lp_coeffs[i].0);
    }
    
    println!("Reflection coefficients:");
    for i in 0..LP_ORDER {
        println!("  k[{}] = {}", i+1, reflection_coeffs[i].0);
    }
    
    // Compare with expected ITU-T reference
    println!("\n=== COMPARISON WITH ITU-T REFERENCE ===");
    println!("Our LP coefficients:    {:?}", lp_coeffs.iter().map(|x| x.0).collect::<Vec<_>>());
    println!("Expected from ITU ref:  [-16384, -7098, 16384, 16384, 16384, 0, 0, 0, 0, 0]");
    
    // Check energy level
    let energy = windowed_autocorr[0].0;
    println!("\nSignal energy R[0] = {}", energy);
    if energy < 1000000 {
        println!("WARNING: Signal energy seems too low for speech!");
    }
}

#[test]
fn test_decode_reference_lsp_indices() {
    println!("Decoding ITU-T reference LSP indices [105, 17, 0, 0]...");
    
    let mut decoder = LSPDecoder::new();
    
    // ITU-T reference indices from ALGTHM.BIT frame 0
    let reference_indices = [105u8, 17u8, 0u8, 0u8];
    
    let decoded_lsp = decoder.decode(&reference_indices);
    
    println!("Reference LSP frequencies: {:?}", 
        decoded_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Also decode our indices for comparison
    let our_indices = [33u8, 8u8, 2u8, 0u8];
    let our_lsp = decoder.decode(&our_indices);
    
    println!("Our LSP frequencies: {:?}", 
        our_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    
    println!("\nDifferences (Ref - Ours):");
    for i in 0..LP_ORDER {
        let diff = decoded_lsp.frequencies[i].0 - our_lsp.frequencies[i].0;
        println!("  LSP[{}]: {} - {} = {}", i, 
            decoded_lsp.frequencies[i].0, our_lsp.frequencies[i].0, diff);
    }
    
    // Convert ITU-T reference LSP back to LP coefficients
    use crate::codecs::g729a::spectral::LSPConverter;
    let converter = LSPConverter::new();
    let reference_lp = converter.lsp_to_lp(&decoded_lsp);
    
    println!("\nITU-T Reference LP coefficients (from LSP conversion):");
    println!("  {:?}", reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Compare with our computed LP coefficients from encoder test
    println!("\nOur computed LP coefficients from signal analysis:");
    println!("  [19320, 2192, 6664, 2784, 1056, 1544, 2976, -192, -1848, -1200] (from test output)");
    
    println!("\nLP coefficient differences (Ref - Ours):");
    let our_lp = [19320, 2192, 6664, 2784, 1056, 1544, 2976, -192, -1848, -1200];
    for i in 0..LP_ORDER {
        let diff = reference_lp.values[i].0 as i32 - our_lp[i] as i32;
        println!("  LP[{}]: {} - {} = {}", i, 
            reference_lp.values[i].0, our_lp[i], diff);
    }
}

#[test]
fn test_levinson_durbin_simple() {
    println!("Testing Levinson-Durbin with simple correlation values...");
    
    let predictor = LinearPredictor::new();
    
    // Create simple test autocorrelation values
    let mut test_corr = vec![Q31(0); LP_ORDER + 1];
    test_corr[0] = Q31(1000000000); // R[0] - significant energy
    test_corr[1] = Q31(500000000);  // R[1] - half correlation
    test_corr[2] = Q31(250000000);  // R[2] - quarter correlation
    // Rest stay zero
    
    println!("Input correlation: {:?}", test_corr.iter().map(|x| x.0).collect::<Vec<_>>());
    
    let start_time = Instant::now();
    let timeout = Duration::from_secs(5); // 5 second timeout
    
    // Run with timeout detection
    let result = std::panic::catch_unwind(|| {
        // This should complete quickly if working correctly
        let (lp_coeffs, reflection_coeffs) = predictor.levinson_durbin(&test_corr);
        (lp_coeffs, reflection_coeffs)
    });
    
    let elapsed = start_time.elapsed();
    println!("Algorithm completed in: {:?}", elapsed);
    
    if elapsed > timeout {
        panic!("Levinson-Durbin algorithm took too long: {:?}", elapsed);
    }
    
    match result {
        Ok((lp_coeffs, reflection_coeffs)) => {
            println!("LP coefficients: {:?}", lp_coeffs.iter().map(|x| x.0).collect::<Vec<_>>());
            println!("Reflection coefficients: {:?}", reflection_coeffs.iter().map(|x| x.0).collect::<Vec<_>>());
            
            // Check that we got non-zero results
            let lp_sum: i32 = lp_coeffs.iter().map(|x| x.0.abs() as i32).sum();
            assert!(lp_sum > 0, "LP coefficients should not all be zero");
        }
        Err(e) => {
            panic!("Levinson-Durbin algorithm panicked: {:?}", e);
        }
    }
}

#[test]
fn test_division_functions() {
    use crate::codecs::g729a::math::fixed_point::{div32_32_q27, div32_32_q31};
    
    println!("Testing division functions...");
    
    // Test simple divisions
    let result_q27 = div32_32_q27(1000000, 2000000);
    println!("div32_32_q27(1000000, 2000000) = {}", result_q27);
    
    let result_q31 = div32_32_q31(1000000, 2000000);
    println!("div32_32_q31(1000000, 2000000) = {}", result_q31);
    
    // Test with realistic autocorrelation values
    let r0 = 1321350144i32;
    let r1 = 1398340619i32;
    
    println!("Testing with r0={}, r1={}", r0, r1);
    
    let start = Instant::now();
    let result = div32_32_q27(r1, r0);
    let elapsed = start.elapsed();
    
    println!("div32_32_q27({}, {}) = {} (took {:?})", r1, r0, result, elapsed);
    
    if elapsed > Duration::from_millis(100) {
        panic!("Division took too long: {:?}", elapsed);
    }
} 

#[test]
fn test_proper_240_sample_windowing() {
    println!("=== TEST PROPER 240-SAMPLE G.729A WINDOWING ===");
    
    // Read the ITU-T test input
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    // Parse all samples
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(Q15(sample));
        }
    }
    
    println!("Total samples: {}, frames: {}", all_samples.len(), all_samples.len() / 80);
    
    // ITU-T reference
    let reference_indices = [105u8, 17u8, 0u8, 0u8];
    let mut decoder = LSPDecoder::new();
    let reference_lsp = decoder.decode(&reference_indices);
    
    use crate::codecs::g729a::spectral::LSPConverter;
    let converter = LSPConverter::new();
    let reference_lp = converter.lsp_to_lp(&reference_lsp);
    
    println!("ITU-T Reference LP coefficients: {:?}", 
        reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Test frame 0 with proper 240-sample windowing
    let frame_idx = 0;
    println!("\n=== TESTING FRAME {} WITH PROPER WINDOWING ===", frame_idx);
    
    // Build 240-sample analysis buffer exactly like the encoder
    let mut analysis_buffer = vec![Q15::ZERO; 240];
    
    // History: For frame 0, use zeros (no previous frames)
    // analysis_buffer[0..120] = zeros (already initialized)
    
    // Current frame: 80 samples starting at frame_idx * 80
    let current_start = frame_idx * 80;
    let current_end = current_start + 80;
    if current_end <= all_samples.len() {
        analysis_buffer[120..200].copy_from_slice(&all_samples[current_start..current_end]);
    }
    
    // Lookahead: 40 samples from the next frame
    let lookahead_start = current_end;
    let lookahead_end = lookahead_start + 40;
    if lookahead_end <= all_samples.len() {
        analysis_buffer[200..240].copy_from_slice(&all_samples[lookahead_start..lookahead_end]);
    }
    
    println!("Analysis buffer built:");
    println!("  History[0..120]: all zeros for frame 0");
    println!("  Current[120..200]: {:?}", &analysis_buffer[120..130].iter().map(|x| x.0).collect::<Vec<_>>());
    println!("  Lookahead[200..240]: {:?}", &analysis_buffer[200..210].iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Apply 240-sample Hamming window (same as encoder)
    use crate::codecs::g729a::tables::window_tables::get_hamming_window;
    let hamming_window = get_hamming_window();
    
    let mut windowed = Vec::new();
    for i in 0..240 {
        let windowed_sample = crate::codecs::g729a::math::fixed_point::mult(
            analysis_buffer[i].0, hamming_window[i].0
        );
        windowed.push(Q15(windowed_sample));
    }
    
    println!("240-sample windowed signal:");
    println!("  First 10: {:?}", &windowed[..10].iter().map(|x| x.0).collect::<Vec<_>>());
    println!("  Current frame start [120..130]: {:?}", &windowed[120..130].iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Compute autocorrelation on 240 samples (same as encoder)
    use crate::codecs::g729a::math::dsp_operations::{autocorrelation, apply_lag_window};
    use crate::codecs::g729a::tables::window_tables::get_lag_window;
    
    let autocorr = autocorrelation(&windowed, LP_ORDER);
    let lag_window = get_lag_window();
    let mut windowed_autocorr = autocorr.clone();
    apply_lag_window(&mut windowed_autocorr, &lag_window);
    
    println!("\nAutocorrelation R[0] to R[5]:");
    for i in 0..=5 {
        println!("  R[{}] = {}", i, windowed_autocorr[i].0);
    }
    
    // Run Levinson-Durbin
    let predictor = LinearPredictor::new();
    let (lp_coeffs, _) = predictor.levinson_durbin(&windowed_autocorr);
    
    println!("\nOur LP coefficients (240-sample): {:?}", 
        lp_coeffs.iter().map(|x| x.0).collect::<Vec<_>>());
    println!("ITU-T Reference LP coefficients: {:?}", 
        reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Calculate distance
    let mut distance = 0i64;
    for i in 0..LP_ORDER {
        let diff = lp_coeffs[i].0 as i64 - reference_lp.values[i].0 as i64;
        distance += diff * diff;
    }
    
    println!("Distance from reference: {}", distance);
    
    if distance < 100000 {
        println!("✅ CLOSE MATCH! Our 240-sample windowing is working!");
    } else if distance < 1000000 {
        println!("🔶 REASONABLE MATCH - may need fine-tuning");
    } else {
        println!("❌ Still quite different - may need deeper investigation");
    }
} 

#[test]
fn test_decode_algthm_bit_frames() {
    println!("=== DECODE ALGTHM.BIT FIRST FEW FRAMES ===");
    
    // Read the ITU-T reference bitstream
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.BIT")
        .expect("Failed to open ALGTHM.BIT");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.BIT");
    
    println!("ALGTHM.BIT file size: {} bytes", buffer.len());
    println!("Total frames: {}", buffer.len() / 10); // 10 bytes per G.729A frame
    
    // Decode first few frames
    for frame_idx in 0..5.min(buffer.len() / 10) {
        let frame_start = frame_idx * 10;
        let frame_bytes = &buffer[frame_start..frame_start + 10];
        
        println!("\nFrame {}: {:02x?}", frame_idx, frame_bytes);
        
        // G.729A bitstream format (80 bits total):
        // LSP: 18 bits (7+5+5+1)
        // Pitch delays: 16 bits (8+5+3) 
        // Fixed codebook: 46 bits (35+13+4+4+4+4+4+4+4+4+3)
        
        // Extract LSP bits from first 3 bytes (18 bits total)
        let mut bit_reader = BitReader::new(frame_bytes);
        
        // LSP Stage 1: 7 bits
        let lsp1 = bit_reader.read_bits(7) as u8;
        
        // LSP Stage 2a: 5 bits 
        let lsp2a = bit_reader.read_bits(5) as u8;
        
        // LSP Stage 2b: 5 bits
        let lsp2b = bit_reader.read_bits(5) as u8;
        
        // LSP Switch: 1 bit
        let lsp_switch = bit_reader.read_bits(1) as u8;
        
        let lsp_indices = [lsp1, lsp2a, lsp2b, lsp_switch];
        
        println!("  LSP indices: [{}, {}, {}, {}]", lsp1, lsp2a, lsp2b, lsp_switch);
        
        // Check if this matches our reference
        if lsp_indices == [105u8, 17u8, 0u8, 0u8] {
            println!("  ⭐ FOUND MATCH! This is the frame that produces reference LSP indices!");
        }
    }
    
    println!("\nLooking for frame with LSP indices [105, 17, 0, 0]...");
}

#[test]
fn test_compare_with_real_frame0_reference() {
    println!("=== COMPARE WITH REAL FRAME 0 REFERENCE ===");
    
    // Read the ITU-T test input
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    // Parse all samples
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(Q15(sample));
        }
    }
    
    // REAL ITU-T reference LSP indices from ALGTHM.BIT frame 0: [16, 22, 22, 1]
    let real_reference_indices = [16u8, 22u8, 22u8, 1u8];
    let mut decoder = LSPDecoder::new();
    let real_reference_lsp = decoder.decode(&real_reference_indices);
    
    use crate::codecs::g729a::spectral::LSPConverter;
    let converter = LSPConverter::new();
    let real_reference_lp = converter.lsp_to_lp(&real_reference_lsp);
    
    println!("REAL ITU-T Frame 0 LSP indices: {:?}", real_reference_indices);
    println!("REAL ITU-T Frame 0 LSP frequencies: {:?}", 
        real_reference_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    println!("REAL ITU-T Frame 0 LP coefficients: {:?}", 
        real_reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Test frame 0 with proper 240-sample windowing and history
    let frame_idx = 0;
    println!("\n=== TESTING OUR FRAME {} ENCODING ===", frame_idx);
    
    // Emulate the encoder: Build 240-sample analysis buffer
    let mut analysis_buffer = vec![Q15::ZERO; 240];
    
    // History: For frame 0, use zeros (no previous frames)
    // analysis_buffer[0..120] = zeros (already initialized)
    
    // Current frame: 80 samples starting at frame_idx * 80
    let current_start = frame_idx * 80;
    let current_end = current_start + 80;
    if current_end <= all_samples.len() {
        analysis_buffer[120..200].copy_from_slice(&all_samples[current_start..current_end]);
    }
    
    // Lookahead: 40 samples from the next frame
    let lookahead_start = current_end;
    let lookahead_end = lookahead_start + 40;
    if lookahead_end <= all_samples.len() {
        analysis_buffer[200..240].copy_from_slice(&all_samples[lookahead_start..lookahead_end]);
    }
    
    // Apply 240-sample Hamming window
    use crate::codecs::g729a::tables::window_tables::get_hamming_window;
    let hamming_window = get_hamming_window();
    
    let mut windowed = Vec::new();
    for i in 0..240 {
        let windowed_sample = crate::codecs::g729a::math::fixed_point::mult(
            analysis_buffer[i].0, hamming_window[i].0
        );
        windowed.push(Q15(windowed_sample));
    }
    
    // LP Analysis
    use crate::codecs::g729a::math::dsp_operations::{autocorrelation, apply_lag_window};
    use crate::codecs::g729a::tables::window_tables::get_lag_window;
    
    let autocorr = autocorrelation(&windowed, LP_ORDER);
    let lag_window = get_lag_window();
    let mut windowed_autocorr = autocorr.clone();
    apply_lag_window(&mut windowed_autocorr, &lag_window);
    
    let predictor = LinearPredictor::new();
    let (lp_coeffs, _) = predictor.levinson_durbin(&windowed_autocorr);
    
    // Convert to LSP
    let lsp = converter.lp_to_lsp(&crate::codecs::g729a::types::LPCoefficients {
        values: lp_coeffs,
        reflection_coeffs: [Q15::ZERO; LP_ORDER],
    });
    
    // Quantize LSP (this is what would be transmitted)
    use crate::codecs::g729a::spectral::LSPQuantizer;
    let mut quantizer = LSPQuantizer::new();
    let quantized = quantizer.quantize(&lsp);
    
    println!("Our Frame 0 LSP indices: {:?}", quantized.indices);
    println!("Our Frame 0 LSP frequencies: {:?}", 
        quantized.reconstructed.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    println!("Our Frame 0 LP coefficients: {:?}", 
        lp_coeffs.iter().map(|x| x.0).collect::<Vec<_>>());
    
    println!("\n=== COMPARISON ===");
    println!("ITU-T Reference: {:?}", real_reference_indices);
    println!("Our Encoder:     {:?}", quantized.indices);
    
    // Calculate distance
    let mut distance = 0i64;
    for i in 0..LP_ORDER {
        let diff = lp_coeffs[i].0 as i64 - real_reference_lp.values[i].0 as i64;
        distance += diff * diff;
    }
    
    println!("LP coefficient distance: {}", distance);
    
    // Check individual indices
    let mut index_matches = 0;
    for i in 0..4 {
        if quantized.indices[i] == real_reference_indices[i] {
            index_matches += 1;
            println!("✅ Index {} matches: {} == {}", i, quantized.indices[i], real_reference_indices[i]);
        } else {
            println!("❌ Index {} differs: {} != {} (diff: {})", 
                i, quantized.indices[i], real_reference_indices[i], 
                quantized.indices[i] as i32 - real_reference_indices[i] as i32);
        }
    }
    
    println!("\nMatching indices: {}/4 ({:.1}%)", index_matches, (index_matches as f32 / 4.0) * 100.0);
    
    if index_matches == 4 {
        println!("🎉 PERFECT MATCH! Our encoder produces the exact ITU-T reference!");
    } else if index_matches >= 2 {
        println!("🔶 REASONABLE MATCH - some indices are correct");
    } else {
        println!("❌ POOR MATCH - needs further debugging");
    }
}

#[test]
fn test_preemphasis_fix() {
    println!("=== TEST PRE-EMPHASIS FILTER FIX ===");
    
    // Read the ITU-T test input
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    // Parse all samples
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    // REAL ITU-T reference LSP indices from ALGTHM.BIT frame 0: [16, 22, 22, 1]
    let real_reference_indices = [16u8, 22u8, 22u8, 1u8];
    let mut decoder = LSPDecoder::new();
    let real_reference_lsp = decoder.decode(&real_reference_indices);
    
    use crate::codecs::g729a::spectral::LSPConverter;
    let converter = LSPConverter::new();
    let real_reference_lp = converter.lsp_to_lp(&real_reference_lsp);
    
    println!("ITU-T Reference Frame 0 LP coefficients: {:?}", 
        real_reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Test frame 0 with PROPER PREPROCESSING
    let frame_idx = 0;
    println!("\n=== TESTING FRAME {} WITH PROPER PREPROCESSING ===", frame_idx);
    
    // Build analysis buffer like the encoder
    let mut analysis_buffer = vec![Q15::ZERO; 240];
    
    // Current frame: 80 samples
    let current_start = frame_idx * 80;
    let current_end = current_start + 80;
    let current_samples = &all_samples[current_start..current_end];
    
    // Lookahead: 40 samples  
    let lookahead_start = current_end;
    let lookahead_end = lookahead_start + 40;
    let lookahead_samples = &all_samples[lookahead_start..lookahead_end];
    
    // Apply PROPER ITU-T preprocessing (pre-emphasis + high-pass)
    use crate::codecs::g729a::signal::Preprocessor;
    let mut preprocessor = Preprocessor::new();
    
    println!("Before preprocessing:");
    println!("  Current frame [0..10]: {:?}", &current_samples[..10]);
    println!("  Lookahead [0..10]: {:?}", &lookahead_samples[..10]);
    
    // Process current frame
    let processed_current = preprocessor.process(current_samples);
    analysis_buffer[120..200].copy_from_slice(&processed_current);
    
    // Process lookahead  
    let processed_lookahead = preprocessor.process(lookahead_samples);
    analysis_buffer[200..240].copy_from_slice(&processed_lookahead);
    
    println!("\nAfter preprocessing:");
    println!("  Processed current [0..10]: {:?}", &processed_current[..10].iter().map(|x| x.0).collect::<Vec<_>>());
    println!("  Processed lookahead [0..10]: {:?}", &processed_lookahead[..10].iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Apply 240-sample Hamming window
    use crate::codecs::g729a::tables::window_tables::get_hamming_window;
    let hamming_window = get_hamming_window();
    
    let mut windowed = Vec::new();
    for i in 0..240 {
        let windowed_sample = crate::codecs::g729a::math::fixed_point::mult(
            analysis_buffer[i].0, hamming_window[i].0
        );
        windowed.push(Q15(windowed_sample));
    }
    
    // LP Analysis with proper preprocessing
    use crate::codecs::g729a::math::dsp_operations::{autocorrelation, apply_lag_window};
    use crate::codecs::g729a::tables::window_tables::get_lag_window;
    
    let autocorr = autocorrelation(&windowed, LP_ORDER);
    let lag_window = get_lag_window();
    let mut windowed_autocorr = autocorr.clone();
    apply_lag_window(&mut windowed_autocorr, &lag_window);
    
    println!("\nAutocorrelation with preprocessing:");
    println!("  R[0] = {}", windowed_autocorr[0].0);
    println!("  R[1] = {}", windowed_autocorr[1].0);
    println!("  R[2] = {}", windowed_autocorr[2].0);
    
    let predictor = LinearPredictor::new();
    let (lp_coeffs, _) = predictor.levinson_durbin(&windowed_autocorr);
    
    // Convert to LSP and quantize
    let lsp = converter.lp_to_lsp(&crate::codecs::g729a::types::LPCoefficients {
        values: lp_coeffs,
        reflection_coeffs: [Q15::ZERO; LP_ORDER],
    });
    
    use crate::codecs::g729a::spectral::LSPQuantizer;
    let mut quantizer = LSPQuantizer::new();
    let quantized = quantizer.quantize(&lsp);
    
    println!("\n=== RESULTS WITH PRE-EMPHASIS ===");
    println!("Our LP coefficients:    {:?}", lp_coeffs.iter().map(|x| x.0).collect::<Vec<_>>());
    println!("ITU-T Reference LP:     {:?}", real_reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    println!("\nOur LSP indices:        {:?}", quantized.indices);
    println!("ITU-T Reference indices: {:?}", real_reference_indices);
    
    // Calculate LP coefficient distance
    let mut lp_distance = 0i64;
    for i in 0..LP_ORDER {
        let diff = lp_coeffs[i].0 as i64 - real_reference_lp.values[i].0 as i64;
        lp_distance += diff * diff;
    }
    
    // Check individual indices
    let mut index_matches = 0;
    for i in 0..4 {
        if quantized.indices[i] == real_reference_indices[i] {
            index_matches += 1;
            println!("✅ Index {} matches: {} == {}", i, quantized.indices[i], real_reference_indices[i]);
        } else {
            println!("❌ Index {} differs: {} != {} (diff: {})", 
                i, quantized.indices[i], real_reference_indices[i], 
                quantized.indices[i] as i32 - real_reference_indices[i] as i32);
        }
    }
    
    println!("\nLP coefficient distance: {}", lp_distance);
    println!("Matching indices: {}/4 ({:.1}%)", index_matches, (index_matches as f32 / 4.0) * 100.0);
    
    if index_matches == 4 {
        println!("🎉 PERFECT MATCH! Pre-emphasis fixed the issue!");
    } else if index_matches >= 2 {
        println!("🔶 SIGNIFICANT IMPROVEMENT with pre-emphasis");
    } else if lp_distance < 500000000 {
        println!("🔶 LP coefficients much closer - pre-emphasis is helping");
    } else {
        println!("❌ Still need more investigation");
    }
}

#[test]
fn test_preemphasis_multiple_frames() {
    println!("=== TEST PRE-EMPHASIS ACROSS MULTIPLE FRAMES ===");
    
    // Read the ITU-T test input
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    // Parse all samples
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    // REAL ITU-T reference LSP indices from ALGTHM.BIT:
    let reference_frames = [
        ([16u8, 22u8, 22u8, 1u8], "Frame 0"),
        ([63u8, 16u8, 1u8, 0u8], "Frame 1"),
        ([64u8, 16u8, 0u8, 1u8], "Frame 2"),
        ([63u8, 16u8, 0u8, 1u8], "Frame 3"),
    ];
    
    use crate::codecs::g729a::spectral::{LSPDecoder, LSPConverter, LSPQuantizer};
    use crate::codecs::g729a::signal::Preprocessor;
    use crate::codecs::g729a::spectral::LinearPredictor;
    use crate::codecs::g729a::math::dsp_operations::{autocorrelation, apply_lag_window};
    use crate::codecs::g729a::tables::window_tables::{get_hamming_window, get_lag_window};
    
    let mut decoder = LSPDecoder::new();
    let converter = LSPConverter::new();
    let predictor = LinearPredictor::new();
    let hamming_window = get_hamming_window();
    let lag_window = get_lag_window();
    
    // Test frames 0-3 to find the correct match
    for frame_idx in 0..4 {
        println!("\n=== TESTING {} WITH PRE-EMPHASIS ===", reference_frames[frame_idx].1);
        
        // Get reference for this frame
        let ref_indices = reference_frames[frame_idx].0;
        let reference_lsp = decoder.decode(&ref_indices);
        let reference_lp = converter.lsp_to_lp(&reference_lsp);
        
        // Check if this frame has enough data
        let current_start = frame_idx * 80;
        let lookahead_end = current_start + 80 + 40;
        if lookahead_end > all_samples.len() {
            println!("  Skipping - not enough data");
            continue;
        }
        
        // Build analysis buffer with history simulation
        let mut analysis_buffer = vec![Q15::ZERO; 240];
        
        // Simulate history: Use previous frames if available
        if frame_idx > 0 {
            let history_start = (frame_idx - 1) * 80;
            let history_end = history_start + 120.min(frame_idx * 80);
            let history_len = history_end - history_start;
            
            // Convert to i16 for preprocessing
            let mut history_samples = Vec::new();
            for i in history_start..history_end {
                history_samples.push(all_samples[i]);
            }
            
            // Process history with a fresh preprocessor state for this frame
            let mut hist_preprocessor = Preprocessor::new();
            let processed_history = hist_preprocessor.process(&history_samples);
            
            // Copy to analysis buffer (fill from the end)
            let copy_start = 120 - history_len;
            analysis_buffer[copy_start..120].copy_from_slice(&processed_history);
        }
        
        // Process current frame and lookahead
        let current_samples = &all_samples[current_start..current_start + 80];
        let lookahead_samples = &all_samples[current_start + 80..current_start + 120];
        
        let mut preprocessor = Preprocessor::new();
        let processed_current = preprocessor.process(current_samples);
        let processed_lookahead = preprocessor.process(lookahead_samples);
        
        analysis_buffer[120..200].copy_from_slice(&processed_current);
        analysis_buffer[200..240].copy_from_slice(&processed_lookahead);
        
        // Check signal energy
        let signal_energy: i64 = analysis_buffer[120..200].iter()
            .map(|&x| (x.0 as i64) * (x.0 as i64))
            .sum();
        
        println!("  Current frame energy: {}", signal_energy);
        
        if signal_energy < 1000 {
            println!("  Skipping low-energy frame");
            continue;
        }
        
        // Apply windowing and LP analysis
        let mut windowed = Vec::new();
        for i in 0..240 {
            let windowed_sample = crate::codecs::g729a::math::fixed_point::mult(
                analysis_buffer[i].0, hamming_window[i].0
            );
            windowed.push(Q15(windowed_sample));
        }
        
        let autocorr = autocorrelation(&windowed, LP_ORDER);
        let mut windowed_autocorr = autocorr.clone();
        apply_lag_window(&mut windowed_autocorr, &lag_window);
        
        let (lp_coeffs, _) = predictor.levinson_durbin(&windowed_autocorr);
        
        // Convert to LSP and quantize
        let lsp = converter.lp_to_lsp(&crate::codecs::g729a::types::LPCoefficients {
            values: lp_coeffs,
            reflection_coeffs: [Q15::ZERO; LP_ORDER],
        });
        
        let mut quantizer = LSPQuantizer::new();
        let quantized = quantizer.quantize(&lsp);
        
        // Compare results
        let mut index_matches = 0;
        for i in 0..4 {
            if quantized.indices[i] == ref_indices[i] {
                index_matches += 1;
            }
        }
        
        let mut lp_distance = 0i64;
        for i in 0..LP_ORDER {
            let diff = lp_coeffs[i].0 as i64 - reference_lp.values[i].0 as i64;
            lp_distance += diff * diff;
        }
        
        println!("  Our indices:    {:?}", quantized.indices);
        println!("  Reference:      {:?}", ref_indices);
        println!("  Index matches:  {}/4", index_matches);
        println!("  LP distance:    {}", lp_distance);
        
        if index_matches >= 3 {
            println!("  🎉 EXCELLENT MATCH for {}!", reference_frames[frame_idx].1);
        } else if index_matches >= 2 {
            println!("  🔶 GOOD MATCH for {}", reference_frames[frame_idx].1);
        } else if lp_distance < 500000000 {
            println!("  🔶 LP coefficients getting closer for {}", reference_frames[frame_idx].1);
        }
    }
}

#[test]
fn test_preemphasis_with_proper_state_management() {
    println!("=== TEST PRE-EMPHASIS WITH PROPER STATE MANAGEMENT ===");
    
    // Read the ITU-T test input
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    // Parse all samples
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    // REAL ITU-T reference LSP indices from ALGTHM.BIT (first 8 frames)
    let reference_frames = [
        ([16u8, 22u8, 22u8, 1u8], "Frame 0"),
        ([63u8, 16u8, 1u8, 0u8], "Frame 1"), 
        ([64u8, 16u8, 0u8, 1u8], "Frame 2"),
        ([63u8, 16u8, 0u8, 1u8], "Frame 3"),
        ([64u8, 17u8, 0u8, 1u8], "Frame 4"),
        ([63u8, 16u8, 1u8, 0u8], "Frame 5"),
        ([64u8, 16u8, 0u8, 1u8], "Frame 6"),
        ([63u8, 16u8, 0u8, 1u8], "Frame 7"),
    ];
    
    use crate::codecs::g729a::spectral::{LSPDecoder, LSPConverter, LSPQuantizer, LinearPredictor};
    use crate::codecs::g729a::signal::Preprocessor;
    use crate::codecs::g729a::math::dsp_operations::{autocorrelation, apply_lag_window};
    use crate::codecs::g729a::tables::window_tables::{get_hamming_window, get_lag_window};
    
    let mut decoder = LSPDecoder::new();
    let converter = LSPConverter::new();
    let predictor = LinearPredictor::new();
    let hamming_window = get_hamming_window();
    let lag_window = get_lag_window();
    
    // CRITICAL: Use a single quantizer instance to maintain state across frames
    let mut quantizer = LSPQuantizer::new();
    let mut preprocessor = Preprocessor::new();
    
    println!("Testing frames 0-7 with proper state management...\n");
    
    let mut total_matches = 0;
    let mut total_frames = 0;
    let mut history_buffer = vec![Q15::ZERO; 120]; // Encoder history state
    
    // Process frames sequentially to maintain proper state
    for frame_idx in 0..8.min(reference_frames.len()) {
        println!("=== PROCESSING {} ===", reference_frames[frame_idx].1);
        
        // Get reference for this frame
        let ref_indices = reference_frames[frame_idx].0;
        let reference_lsp = decoder.decode(&ref_indices);
        let reference_lp = converter.lsp_to_lp(&reference_lsp);
        
        // Check if this frame has enough data
        let current_start = frame_idx * 80;
        let lookahead_end = current_start + 80 + 40;
        if lookahead_end > all_samples.len() {
            println!("  Skipping - not enough data");
            continue;
        }
        
        // Build analysis buffer exactly like the encoder
        let mut analysis_buffer = vec![Q15::ZERO; 240];
        
        // Copy history (from previous frames)
        analysis_buffer[..120].copy_from_slice(&history_buffer);
        
        // Process current frame and lookahead with pre-emphasis
        let current_samples = &all_samples[current_start..current_start + 80];
        let lookahead_samples = &all_samples[current_start + 80..current_start + 120];
        
        let processed_current = preprocessor.process(current_samples);
        let processed_lookahead = preprocessor.process(lookahead_samples);
        
        analysis_buffer[120..200].copy_from_slice(&processed_current);
        analysis_buffer[200..240].copy_from_slice(&processed_lookahead);
        
        // Update history for next frame (last 120 samples)
        history_buffer.clear();
        history_buffer.extend_from_slice(&analysis_buffer[120..240]);
        
        // Check signal energy
        let signal_energy: i64 = analysis_buffer[120..200].iter()
            .map(|&x| (x.0 as i64) * (x.0 as i64))
            .sum();
        
        println!("  Current frame energy: {}", signal_energy);
        
        // Apply windowing and LP analysis
        let mut windowed = Vec::new();
        for i in 0..240 {
            let windowed_sample = crate::codecs::g729a::math::fixed_point::mult(
                analysis_buffer[i].0, hamming_window[i].0
            );
            windowed.push(Q15(windowed_sample));
        }
        
        let autocorr = autocorrelation(&windowed, LP_ORDER);
        let mut windowed_autocorr = autocorr.clone();
        apply_lag_window(&mut windowed_autocorr, &lag_window);
        
        let (lp_coeffs, _) = predictor.levinson_durbin(&windowed_autocorr);
        
        // Convert to LSP and quantize with MAINTAINED STATE
        let lsp = converter.lp_to_lsp(&crate::codecs::g729a::types::LPCoefficients {
            values: lp_coeffs,
            reflection_coeffs: [Q15::ZERO; LP_ORDER],
        });
        
        let quantized = quantizer.quantize(&lsp); // State maintained across calls!
        
        // Compare results
        let mut index_matches = 0;
        for i in 0..4 {
            if quantized.indices[i] == ref_indices[i] {
                index_matches += 1;
            }
        }
        
        let mut lp_distance = 0i64;
        for i in 0..LP_ORDER {
            let diff = lp_coeffs[i].0 as i64 - reference_lp.values[i].0 as i64;
            lp_distance += diff * diff;
        }
        
        println!("  Our indices:    {:?}", quantized.indices);
        println!("  Reference:      {:?}", ref_indices);
        println!("  Index matches:  {}/4 ({:.1}%)", index_matches, (index_matches as f32 / 4.0) * 100.0);
        println!("  LP distance:    {}", lp_distance);
        
        total_matches += index_matches;
        total_frames += 1;
        
        if index_matches >= 3 {
            println!("  🎉 EXCELLENT MATCH for {}!", reference_frames[frame_idx].1);
        } else if index_matches >= 2 {
            println!("  🔶 GOOD MATCH for {}", reference_frames[frame_idx].1);
        } else if index_matches >= 1 {
            println!("  🔶 SOME PROGRESS for {}", reference_frames[frame_idx].1);
        } else if lp_distance < 500000000 {
            println!("  🔶 LP coefficients improving for {}", reference_frames[frame_idx].1);
        }
        
        println!();
    }
    
    println!("=== FINAL RESULTS WITH PRE-EMPHASIS + PROPER STATE ===");
    println!("Total index matches: {}/{} ({:.1}%)", 
        total_matches, total_frames * 4, (total_matches as f32 / (total_frames * 4) as f32) * 100.0);
    
    if total_matches >= total_frames * 3 {
        println!("🎉 EXCELLENT OVERALL PERFORMANCE!");
    } else if total_matches >= total_frames * 2 {
        println!("🔶 GOOD OVERALL PERFORMANCE - Getting close!");
    } else if total_matches >= total_frames {
        println!("🔶 SIGNIFICANT IMPROVEMENT - Pre-emphasis is working!");
    } else {
        println!("📊 Pre-emphasis showing improvement but more work needed");
    }
}

#[test]
fn test_encoder_with_preemphasis() {
    println!("=== TEST FULL G.729A ENCODER WITH PRE-EMPHASIS ===");
    
    // Read the ITU-T test input
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    // Parse samples in chunks of 80 for frames
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    // Read reference bitstream
    let mut bit_file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.BIT")
        .expect("Failed to open ALGTHM.BIT");
    let mut bit_buffer = Vec::new();
    bit_file.read_to_end(&mut bit_buffer).expect("Failed to read ALGTHM.BIT");
    
    // Parse reference frames (10 bytes each)
    let mut reference_frames = Vec::new();
    for frame_idx in 0..(bit_buffer.len() / 10).min(8) {
        let frame_start = frame_idx * 10;
        let frame_bytes = &bit_buffer[frame_start..frame_start + 10];
        
        // Extract LSP indices (first 18 bits: 7+5+5+1)
        let lsp0 = frame_bytes[0] & 0x7F; // First 7 bits
        let lsp1 = ((frame_bytes[0] & 0x80) >> 7) | ((frame_bytes[1] & 0x0F) << 1); // Next 5 bits
        let lsp2 = (frame_bytes[1] & 0xF0) >> 4; // Next 4 bits
        let lsp2_extra = (frame_bytes[2] & 0x01) << 4; // 1 more bit
        let lsp2_full = lsp2 | lsp2_extra; // 5 bits total
        let lsp3 = (frame_bytes[2] & 0x02) >> 1; // Next 1 bit
        
        reference_frames.push([lsp0, lsp1, lsp2_full, lsp3]);
    }
    
    println!("Reference frames extracted:");
    for (i, frame) in reference_frames.iter().enumerate() {
        println!("  Frame {}: {:?}", i, frame);
    }
    
    // Create encoder with pre-emphasis
    use crate::codecs::g729a::codec::encoder::G729AEncoder;
    use crate::codecs::g729a::types::AudioFrame;
    
    let mut encoder = G729AEncoder::new();
    
    let mut total_matches = 0;
    let mut total_frames = 0;
    
    println!("\n=== ENCODING WITH REAL G.729A ENCODER (PRE-EMPHASIS ENABLED) ===");
    
    // Encode first 8 frames
    for frame_idx in 0..8.min(reference_frames.len()) {
        let start_sample = frame_idx * 80;
        let end_sample = start_sample + 80;
        
        if end_sample > all_samples.len() {
            println!("Frame {}: Not enough samples", frame_idx);
            break;
        }
        
        // Prepare current frame
        let frame_samples = &all_samples[start_sample..end_sample];
        let audio_frame = AudioFrame::from_pcm(&frame_samples.to_vec()).unwrap();
        
        // Prepare lookahead (next 40 samples or zeros)
        let lookahead_start = end_sample;
        let lookahead_end = (lookahead_start + 40).min(all_samples.len());
        let mut lookahead = vec![0i16; 40];
        
        if lookahead_start < all_samples.len() {
            let available = lookahead_end - lookahead_start;
            lookahead[..available].copy_from_slice(&all_samples[lookahead_start..lookahead_end]);
        }
        
        // Encode frame
        match encoder.encode_frame_with_lookahead(&audio_frame, &lookahead) {
            Ok(encoded_bytes) => {
                // Extract LSP indices from encoded frame (same parsing as reference)
                let our_lsp0 = encoded_bytes[0] & 0x7F;
                let our_lsp1 = ((encoded_bytes[0] & 0x80) >> 7) | ((encoded_bytes[1] & 0x0F) << 1);
                let our_lsp2 = (encoded_bytes[1] & 0xF0) >> 4;
                let our_lsp2_extra = (encoded_bytes[2] & 0x01) << 4;
                let our_lsp2_full = our_lsp2 | our_lsp2_extra;
                let our_lsp3 = (encoded_bytes[2] & 0x02) >> 1;
                
                let our_indices = [our_lsp0, our_lsp1, our_lsp2_full, our_lsp3];
                let ref_indices = reference_frames[frame_idx];
                
                // Compare indices
                let mut matches = 0;
                for i in 0..4 {
                    if our_indices[i] == ref_indices[i] {
                        matches += 1;
                    }
                }
                
                total_matches += matches;
                total_frames += 1;
                
                println!("Frame {}: Our={:?}, Ref={:?}, Matches={}/4 ({:.1}%)", 
                    frame_idx, our_indices, ref_indices, matches, (matches as f32 / 4.0) * 100.0);
                
                if matches >= 3 {
                    println!("  🎉 EXCELLENT! Frame {} is very close to reference", frame_idx);
                } else if matches >= 2 {
                    println!("  🔶 GOOD! Frame {} shows significant improvement", frame_idx);
                } else if matches >= 1 {
                    println!("  🔶 PROGRESS! Frame {} has some matches", frame_idx);
                }
            }
            Err(e) => {
                println!("Frame {}: Encoding failed: {:?}", frame_idx, e);
            }
        }
    }
    
    println!("\n=== FINAL ENCODER RESULTS WITH PRE-EMPHASIS ===");
    let total_possible = total_frames * 4;
    let compliance_rate = if total_possible > 0 {
        (total_matches as f32 / total_possible as f32) * 100.0
    } else {
        0.0
    };
    
    println!("Total frames tested: {}", total_frames);
    println!("Total index matches: {}/{} ({:.1}%)", total_matches, total_possible, compliance_rate);
    
    if compliance_rate >= 75.0 {
        println!("🎉 EXCELLENT COMPLIANCE! Pre-emphasis has nearly fixed the issue!");
    } else if compliance_rate >= 50.0 {
        println!("🔶 GOOD COMPLIANCE! Pre-emphasis is making major improvements!");
    } else if compliance_rate >= 25.0 {
        println!("🔶 SIGNIFICANT IMPROVEMENT! Pre-emphasis is working!");
    } else if compliance_rate >= 10.0 {
        println!("📊 MEASURABLE IMPROVEMENT! Pre-emphasis is helping!");
    } else if compliance_rate > 0.0 {
        println!("📊 SOME IMPROVEMENT! Pre-emphasis is having an effect!");
    } else {
        println!("❌ No improvement detected - need more investigation");
    }
    
    // Overall assessment
    if total_matches > 0 {
        println!("\n✅ PRE-EMPHASIS FILTER IS WORKING!");
        println!("The improvements show that the preprocessing pipeline fix is effective.");
        println!("Further tuning of LSP quantization and other parameters may yield even better results.");
    }
}

#[test]
fn test_debug_lp_to_lsp_conversion() {
    println!("=== DEBUG LP→LSP CONVERSION ALGORITHM ===");
    
    // Read Frame 0 from ALGTHM.IN and compute LP coefficients
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    // Build Frame 0 analysis buffer (with pre-emphasis)
    let mut analysis_buffer = vec![0i16; 240];
    for i in 0..80 {
        if i < all_samples.len() {
            analysis_buffer[120 + i] = all_samples[i]; // Current frame
        }
    }
    for i in 0..40 {
        if 80 + i < all_samples.len() {
            analysis_buffer[200 + i] = all_samples[80 + i]; // Lookahead
        }
    }
    
    // Apply pre-emphasis filter
    let mut preprocessor = crate::codecs::g729a::signal::preprocessor::Preprocessor::new();
    let processed_samples = preprocessor.process(&analysis_buffer);
    
    // Apply Hamming window and compute LP
    let hamming_window = crate::codecs::g729a::tables::window_tables::get_hamming_window();
    let mut windowed = Vec::new();
    for i in 0..240 {
        let sample = crate::codecs::g729a::math::fixed_point::mult(processed_samples[i].0, hamming_window[i].0);
        windowed.push(Q15(sample));
    }
    
    // Compute autocorrelation, apply lag window, and run Levinson-Durbin
    let mut correlations = crate::codecs::g729a::math::dsp_operations::autocorrelation(&windowed, LP_ORDER);
    let lag_window = crate::codecs::g729a::tables::window_tables::get_lag_window();
    crate::codecs::g729a::math::dsp_operations::apply_lag_window(&mut correlations, &lag_window);
    
    let linear_predictor = crate::codecs::g729a::spectral::linear_prediction::LinearPredictor::new();
    let (lp_coeffs, _) = linear_predictor.levinson_durbin(&correlations);
    
    println!("Our computed LP coefficients: {:?}", lp_coeffs.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Now decode the ITU-T reference LSP indices to get reference LP coefficients
    let real_reference_indices = [16u8, 22u8, 22u8, 1u8];
    let mut lsp_decoder = crate::codecs::g729a::spectral::quantizer::LSPDecoder::new();
    let decoded_lsp = lsp_decoder.decode(&real_reference_indices);
    
    println!("ITU-T reference LSP: {:?}", decoded_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Convert ITU-T reference LSP back to LP coefficients
    let lsp_converter = crate::codecs::g729a::spectral::lsp_converter::LSPConverter::new();
    let reference_lp = lsp_converter.lsp_to_lp(&decoded_lsp);
    
    println!("ITU-T reference LP coefficients: {:?}", reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Now compare our LP→LSP conversion
    let lp_struct = crate::codecs::g729a::types::LPCoefficients {
        values: lp_coeffs,
        reflection_coeffs: [Q15::ZERO; LP_ORDER],
    };
    
    println!("\n=== POLYNOMIAL FORMATION DEBUG ===");
    
    // Debug polynomial formation step-by-step
    let f1_coeffs = crate::codecs::g729a::math::polynomial::form_sum_polynomial_q12(&lp_coeffs);
    let f2_coeffs = crate::codecs::g729a::math::polynomial::form_difference_polynomial_q12(&lp_coeffs);
    
    println!("Our F1 polynomial (Q15): {:?}", f1_coeffs.iter().map(|&x| x).collect::<Vec<_>>());
    println!("Our F2 polynomial (Q15): {:?}", f2_coeffs.iter().map(|&x| x).collect::<Vec<_>>());
    
    // Compare with polynomial formation from ITU-T reference LP
    let ref_f1_coeffs = crate::codecs::g729a::math::polynomial::form_sum_polynomial_q12(&reference_lp.values);
    let ref_f2_coeffs = crate::codecs::g729a::math::polynomial::form_difference_polynomial_q12(&reference_lp.values);
    
    println!("ITU-T F1 polynomial (Q15): {:?}", ref_f1_coeffs.iter().map(|&x| x).collect::<Vec<_>>());
    println!("ITU-T F2 polynomial (Q15): {:?}", ref_f2_coeffs.iter().map(|&x| x).collect::<Vec<_>>());
    
    // Check polynomial differences
    let mut f1_diff = 0i64;
    let mut f2_diff = 0i64;
    for i in 0..6 {
        f1_diff += ((f1_coeffs[i] - ref_f1_coeffs[i]) as i64).abs();
        f2_diff += ((f2_coeffs[i] - ref_f2_coeffs[i]) as i64).abs();
    }
    
    println!("Polynomial differences: F1={}, F2={}", f1_diff, f2_diff);
    
    // Convert our LP to LSP and compare
    let our_lsp = lsp_converter.lp_to_lsp(&lp_struct);
    
    println!("\n=== LSP COMPARISON ===");
    println!("Our LSP:     {:?}", our_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    println!("ITU-T LSP:   {:?}", decoded_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Calculate LSP frequency differences
    let mut lsp_diff = 0i64;
    for i in 0..LP_ORDER {
        lsp_diff += ((our_lsp.frequencies[i].0 as i64) - (decoded_lsp.frequencies[i].0 as i64)).abs();
    }
    
    println!("Total LSP frequency difference: {}", lsp_diff);
    
    // The problem should now be clear from this detailed comparison
}

// Simple bit reader for parsing bitstream
struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }
    
    fn read_bits(&mut self, num_bits: u8) -> u32 {
        let mut result = 0u32;
        
        for _ in 0..num_bits {
            if self.byte_pos >= self.data.len() {
                break;
            }
            
            let bit = (self.data[self.byte_pos] >> (7 - self.bit_pos)) & 1;
            result = (result << 1) | (bit as u32);
            
            self.bit_pos += 1;
            if self.bit_pos == 8 {
                self.bit_pos = 0;
                self.byte_pos += 1;
            }
        }
        
        result
    }
} 

#[test]
fn test_full_encoder_itu_t_compliance() {
    println!("=== TEST FULL G.729A ENCODER ITU-T COMPLIANCE ===");
    
    // Read Frame 0 from ALGTHM.IN
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    println!("Loaded {} samples from ALGTHM.IN", all_samples.len());
    
    // Create ITU-T reference LSP indices for comparison
    let reference_indices = [16u8, 22u8, 22u8, 1u8]; // Frame 0 from ALGTHM.BIT
    
    // Create a fresh G.729A encoder with ITU-T initialization
    let mut encoder = crate::codecs::g729a::codec::encoder::G729AEncoder::new();
    
    // Build Frame 0 (samples 0-79) and Frame 1 lookahead (samples 80-119)
    let mut frame_0 = [0i16; 80];
    let mut lookahead = [0i16; 40];
    
    for i in 0..80 {
        if i < all_samples.len() {
            frame_0[i] = all_samples[i];
        }
    }
    
    for i in 0..40 {
        if 80 + i < all_samples.len() {
            lookahead[i] = all_samples[80 + i];
        }
    }
    
    println!("Frame 0 samples [0..10]: {:?}", &frame_0[0..10]);
    println!("Frame 0 energy: {}", frame_0.iter().map(|&x| (x as i64) * (x as i64)).sum::<i64>());
    
    // Create AudioFrame
    let audio_frame = crate::codecs::g729a::types::AudioFrame {
        samples: frame_0,
        timestamp: 0,
    };
    
    // Encode Frame 0 with proper lookahead
    println!("\n=== ENCODING FRAME 0 ===");
    let encoded_result = encoder.encode_frame_with_lookahead(&audio_frame, &lookahead);
    
    match encoded_result {
        Ok(encoded_bytes) => {
            // Unpack the bitstream to get encoded parameters
            let decoded_params = crate::codecs::g729a::codec::bitstream::unpack_frame(&encoded_bytes);
            
            println!("Successfully encoded Frame 0!");
            println!("Encoded LSP indices: {:?}", decoded_params.lsp_indices);
            println!("Reference LSP indices: {:?}", reference_indices);
            
            // Compare LSP indices
            let mut matches = 0;
            for i in 0..4 {
                if decoded_params.lsp_indices[i] == reference_indices[i] {
                    matches += 1;
                    println!("✓ LSP[{}]: {} (match)", i, decoded_params.lsp_indices[i]);
                } else {
                    println!("✗ LSP[{}]: {} vs {} (reference)", i, decoded_params.lsp_indices[i], reference_indices[i]);
                }
            }
            
            let compliance_percentage = (matches as f64 / 4.0) * 100.0;
            println!("\n🎯 FRAME 0 COMPLIANCE: {:.1}% ({}/{} indices match)", 
                compliance_percentage, matches, 4);
            
            // Also check other parameters
            println!("\nOther encoded parameters:");
            println!("Pitch delays: {:?}", decoded_params.pitch_delays);
            println!("Fixed codebook indices: {:?}", decoded_params.fixed_codebook_indices);
            println!("Gain indices: {:?}", decoded_params.gain_indices);
            
            // If we get 100% compliance, we've solved it!
            if matches == 4 {
                println!("🎉 PERFECT COMPLIANCE ACHIEVED! 🎉");
            } else {
                println!("❌ Still not perfect compliance. Need to investigate further.");
            }
        },
        Err(e) => {
            println!("❌ Encoding failed: {:?}", e);
            panic!("Encoder failed to process Frame 0");
        }
    }
}

#[test]
fn test_find_correct_frame_alignment() {
    println!("=== FIND CORRECT FRAME ALIGNMENT ===");
    
    // Read all samples from ALGTHM.IN
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    println!("Loaded {} samples from ALGTHM.IN", all_samples.len());
    
    // Target reference from ALGTHM.BIT Frame 0
    let target_indices = [16u8, 22u8, 22u8, 1u8];
    
    // Test frames 0-10 to find which one matches
    for frame_num in 0..=10 {
        let frame_start = frame_num * 80;
        let lookahead_start = frame_start + 80;
        
        if frame_start + 80 > all_samples.len() {
            break;
        }
        
        // Build frame samples
        let mut frame_samples = [0i16; 80];
        let mut lookahead = [0i16; 40];
        
        for i in 0..80 {
            if frame_start + i < all_samples.len() {
                frame_samples[i] = all_samples[frame_start + i];
            }
        }
        
        for i in 0..40 {
            if lookahead_start + i < all_samples.len() {
                lookahead[i] = all_samples[lookahead_start + i];
            }
        }
        
        // Calculate frame energy for diagnostic purposes
        let frame_energy: i64 = frame_samples.iter().map(|&x| (x as i64) * (x as i64)).sum();
        let max_sample = frame_samples.iter().map(|&x| x.abs()).max().unwrap_or(0);
        
        println!("\nFrame {}: energy={}, max_sample={}", frame_num, frame_energy, max_sample);
        println!("  Samples [0..10]: {:?}", &frame_samples[0..10]);
        
        // Skip silent frames (likely won't match active speech reference)
        if frame_energy < 1000000 {
            println!("  -> Skipping silent frame");
            continue;
        }
        
        // Create a fresh encoder for each test
        let mut encoder = crate::codecs::g729a::codec::encoder::G729AEncoder::new();
        
        let audio_frame = crate::codecs::g729a::types::AudioFrame {
            samples: frame_samples,
            timestamp: frame_num as u64 * 80,
        };
        
        // Encode the frame
        match encoder.encode_frame_with_lookahead(&audio_frame, &lookahead) {
            Ok(encoded_bytes) => {
                let decoded_params = crate::codecs::g729a::codec::bitstream::unpack_frame(&encoded_bytes);
                
                // Check for matches
                let mut matches = 0;
                for i in 0..4 {
                    if decoded_params.lsp_indices[i] == target_indices[i] {
                        matches += 1;
                    }
                }
                
                let compliance = (matches as f64 / 4.0) * 100.0;
                println!("  -> LSP indices: {:?} ({}% match)", decoded_params.lsp_indices, compliance);
                
                if matches >= 2 {
                    println!("  🎯 POTENTIAL MATCH! Frame {} has {}% compliance", frame_num, compliance);
                    if matches == 4 {
                        println!("  🎉 PERFECT MATCH FOUND! Frame {} produces exact reference indices!", frame_num);
                        break;
                    }
                }
            },
            Err(e) => {
                println!("  -> Encoding failed: {:?}", e);
            }
        }
    }
    
    println!("\n=== FRAME ALIGNMENT SEARCH COMPLETE ===");
}

#[test]
fn test_decoder_reference_reconstruction() {
    println!("=== TEST DECODER REFERENCE RECONSTRUCTION ===");
    
    // ITU-T reference indices from ALGTHM.BIT Frame 0
    let reference_indices = [16u8, 22u8, 22u8, 1u8];
    
    println!("Testing decoder reconstruction of reference indices: {:?}", reference_indices);
    
    // Create a fresh decoder
    let mut decoder = crate::codecs::g729a::spectral::quantizer::LSPDecoder::new();
    
    // Decode the reference indices
    let decoded_lsp = decoder.decode(&reference_indices);
    
    println!("Decoded LSP frequencies: {:?}", decoded_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Convert decoded LSP back to LP coefficients
    let lsp_converter = crate::codecs::g729a::spectral::lsp_converter::LSPConverter::new();
    let decoded_lp = lsp_converter.lsp_to_lp(&decoded_lsp);
    
    println!("Decoded LP coefficients: {:?}", decoded_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Now test the round-trip: encode these LP coefficients back
    let re_encoded_lsp = lsp_converter.lp_to_lsp(&decoded_lp);
    
    println!("Re-encoded LSP frequencies: {:?}", re_encoded_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Try to quantize the re-encoded LSP
    let mut quantizer = crate::codecs::g729a::spectral::quantizer::LSPQuantizer::new();
    let quantized = quantizer.quantize(&re_encoded_lsp);
    
    println!("Re-quantized indices: {:?}", quantized.indices);
    
    // Check if we get back the original indices
    let mut matches = 0;
    for i in 0..4 {
        if quantized.indices[i] == reference_indices[i] {
            matches += 1;
            println!("✓ Index[{}]: {} (match)", i, quantized.indices[i]);
        } else {
            println!("✗ Index[{}]: {} vs {} (reference)", i, quantized.indices[i], reference_indices[i]);
        }
    }
    
    let round_trip_compliance = (matches as f64 / 4.0) * 100.0;
    println!("\n🔄 ROUND-TRIP COMPLIANCE: {:.1}% ({}/{} indices match)", 
        round_trip_compliance, matches, 4);
    
    if matches == 4 {
        println!("🎉 PERFECT ROUND-TRIP! Our decoder and encoder are consistent!");
    } else {
        println!("❌ Round-trip failed. This indicates encoder/decoder mismatch.");
    }
}

#[test]
fn test_quantizer_with_exact_reference_lsp() {
    println!("=== TEST QUANTIZER WITH EXACT REFERENCE LSP ===");
    
    // From the decoder test, we know reference indices [16,22,22,1] decode to:
    // LSP frequencies: [5105, 9458, 17860, 28222, 32767, 23040, 29359, 32451, 32136, 32411]
    let reference_lsp_frequencies = [5105, 9458, 17860, 28222, 32767, 23040, 29359, 32451, 32136, 32411];
    let reference_indices = [16u8, 22u8, 22u8, 1u8];
    
    println!("Testing with exact reference LSP frequencies: {:?}", &reference_lsp_frequencies[..5]);
    
    // Create LSP parameters with these exact frequencies
    let reference_lsp = crate::codecs::g729a::types::LSPParameters {
        frequencies: reference_lsp_frequencies.map(|f| crate::codecs::g729a::types::Q15(f)),
    };
    
    // Create a fresh quantizer
    let mut quantizer = crate::codecs::g729a::spectral::quantizer::LSPQuantizer::new();
    
    // Quantize these exact reference LSP frequencies
    let quantized = quantizer.quantize(&reference_lsp);
    
    println!("Our quantized indices: {:?}", quantized.indices);
    println!("Reference indices:     {:?}", reference_indices);
    
    // Check for perfect match
    let mut matches = 0;
    for i in 0..4 {
        if quantized.indices[i] == reference_indices[i] {
            matches += 1;
            println!("✓ Index[{}]: {} (PERFECT)", i, quantized.indices[i]);
        } else {
            println!("✗ Index[{}]: {} vs {} (reference)", i, quantized.indices[i], reference_indices[i]);
        }
    }
    
    let compliance = (matches as f64 / 4.0) * 100.0;
    println!("\n🎯 EXACT LSP COMPLIANCE: {:.1}% ({}/{} indices match)", 
        compliance, matches, 4);
    
    if matches == 4 {
        println!("🎉 PERFECT! Our quantizer is 100% accurate when given exact reference LSP!");
        println!("    This confirms the issue is in our LP analysis → LSP conversion pipeline.");
    } else {
        println!("❌ Quantizer error: Even with exact reference LSP, we don't get perfect indices.");
        println!("    This indicates a bug in our quantization implementation.");
    }
}

#[test]
fn test_reverse_engineer_reference() {
    println!("=== REVERSE ENGINEER REFERENCE ===");
    
    // The ITU-T expects indices [1, 105, 17, 0]
    // Let's see what codebook values these correspond to
    use crate::codecs::g729a::tables::{LSP_CB1, LSP_CB2};
    
    let l0 = 1;
    let l1 = 105;
    let l2 = 17; 
    let l3 = 0;
    
    println!("Reference indices: L0={}, L1={}, L2={}, L3={}", l0, l1, l2, l3);
    
    // Codebook values
    println!("\nCodebook entries:");
    println!("L1[{}]: {:?}", l1, LSP_CB1[l1]);
    println!("L2[{}]: {:?}", l2, &LSP_CB2[l2][0..5]);
    println!("L3[{}]: {:?}", l3, &LSP_CB2[l3][5..10]);
    
    // Quantizer output (before MA prediction)
    let mut quantizer_output = [0i16; 10];
    for i in 0..5 {
        quantizer_output[i] = LSP_CB1[l1][i] + LSP_CB2[l2][i];
    }
    for i in 5..10 {
        quantizer_output[i] = LSP_CB1[l1][i] + LSP_CB2[l3][i];
    }
    
    println!("\nQuantizer output (sum of codebooks): {:?}", quantizer_output);
    
    // This is the LSF that should be found by subtracting MA prediction from input LSF
    // So: input_LSF = quantizer_output + MA_prediction
    
    // Initial prev_lsf values
    let prev_lsf = [2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396];
    
    // MA predictors for L0=1
    let ma_pred = [
        [7733, 7880, 8188, 8175, 8247, 8490, 8637, 8601, 8359, 7569],
        [4210, 3031, 2552, 3473, 3876, 3853, 4184, 4154, 3909, 3968],
        [3214, 1930, 1313, 2143, 2493, 2385, 2755, 2706, 2542, 2919],
        [3024, 1592, 940, 1631, 1723, 1579, 2034, 2084, 1913, 2601],
    ];
    
    println!("\nWhat input LSF would produce these indices?");
    for i in 0..5 {
        let mut ma_sum = 0i64;
        for j in 0..4 {
            ma_sum += (prev_lsf[i] as i64) * (ma_pred[j][i] as i64);
        }
        // The input LSF should satisfy:
        // (input_LSF << 15) - ma_sum = quantizer_output * MAPredictorSum
        let ma_pred_sum = 14585i32; // For L0=1, coeff 0
        let target_acc = (quantizer_output[i] as i32) * ma_pred_sum;
        let input_lsf = ((target_acc as i64 + ma_sum) >> 15) as i16;
        
        println!("  LSF[{}]: quantizer_out={} -> needs input_lsf≈{}", 
            i, quantizer_output[i], input_lsf);
    }
}

#[test]
fn test_encoder_against_correct_algthm_reference() {
    println!("=== TEST ENCODER AGAINST CORRECT ALGTHM.BIT FRAME 0 ===");
    
    // Read ALGTHM.IN 
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    // CORRECT ITU-T reference from ALGTHM.BIT Frame 0: [1, 105, 17, 0]
    let correct_reference_indices = [1u8, 105u8, 17u8, 0u8];
    
    println!("CORRECT ALGTHM.BIT Frame 0 LSP indices: {:?}", correct_reference_indices);
    
    // Create encoder and encode Frame 0
    use crate::codecs::g729a::{G729AEncoder, AudioFrame};
    
    let mut encoder = G729AEncoder::new();
    
    // Build Frame 0 (samples 0-79) and Frame 1 lookahead (samples 80-119)
    let mut frame_0 = [0i16; 80];
    let mut lookahead = [0i16; 40];
    
    for i in 0..80 {
        if i < all_samples.len() {
            frame_0[i] = all_samples[i];
        }
    }
    for i in 0..40 {
        if 80 + i < all_samples.len() {
            lookahead[i] = all_samples[80 + i];
        }
    }
    
    let audio_frame = AudioFrame {
        samples: frame_0,
        timestamp: 0,
    };
    
    // Encode Frame 0
    match encoder.encode_frame_with_lookahead(&audio_frame, &lookahead) {
        Ok(encoded_bytes) => {
            println!("Successfully encoded Frame 0: {:02X?}", encoded_bytes);
            
            // Parse our encoded output to extract LSP indices
            use crate::codecs::g729a::codec::bitstream::unpack_frame;
            let decoded_params = unpack_frame(&encoded_bytes);
            
            println!("Our LSP indices: {:?}", decoded_params.lsp_indices);
            println!("Correct reference: {:?}", correct_reference_indices);
            
            // Check individual matches
            let mut matches = 0;
            for i in 0..4 {
                if decoded_params.lsp_indices[i] == correct_reference_indices[i] {
                    matches += 1;
                    println!("  ✅ Index {}: {} matches", i, decoded_params.lsp_indices[i]);
                } else {
                    println!("  ❌ Index {}: {} ≠ {}", i, decoded_params.lsp_indices[i], correct_reference_indices[i]);
                }
            }
            
            let compliance = (matches as f64 / 4.0) * 100.0;
            println!("\n🎯 TRUE COMPLIANCE: {:.1}% ({}/4 LSP indices match)", compliance, matches);
            
            if matches == 4 {
                println!("🎉 PERFECT MATCH! Our encoder produces the correct ALGTHM.BIT Frame 0!");
            } else if matches >= 2 {
                println!("🔶 SIGNIFICANT PROGRESS - LSP core working, need fine-tuning");
            } else {
                println!("❌ Still fundamental LSP issues to resolve");
            }
        }
        Err(e) => {
            println!("❌ Encoding failed: {:?}", e);
        }
    }
}

#[test]
fn test_reference_lsp_to_lp_vs_our_lp() {
    println!("=== COMPARE REFERENCE LSP→LP vs OUR LP ANALYSIS ===");
    
    // Decode the correct ALGTHM.BIT Frame 0 LSP indices [16, 22, 22, 1]
    let reference_indices = [16u8, 22u8, 22u8, 1u8];
    
    use crate::codecs::g729a::spectral::{LSPDecoder, LSPConverter};
    let mut decoder = LSPDecoder::new();
    let reference_lsp = decoder.decode(&reference_indices);
    
    println!("ITU-T Reference LSP indices: {:?}", reference_indices);
    println!("ITU-T Reference LSP frequencies: {:?}", 
        reference_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Convert reference LSP to LP coefficients using our converter
    let converter = LSPConverter::new();
    let reference_lp = converter.lsp_to_lp(&reference_lsp);
    
    println!("ITU-T Reference LP coefficients (from LSP): {:?}", 
        reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Now run our LP analysis on Frame 0 and compare
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    // Build Frame 0 analysis buffer exactly like the encoder
    let mut analysis_buffer = vec![0i16; 240];
    
    // Current frame: 80 samples starting at frame_idx * 80
    for i in 0..80 {
        if i < all_samples.len() {
            analysis_buffer[120 + i] = all_samples[i];
        }
    }
    // Lookahead: 40 samples  
    for i in 0..40 {
        if 80 + i < all_samples.len() {
            analysis_buffer[200 + i] = all_samples[80 + i];
        }
    }
    
    // Apply preprocessing exactly like encoder
    use crate::codecs::g729a::signal::Preprocessor;
    let mut preprocessor = Preprocessor::new();
    let processed_samples = preprocessor.process(&analysis_buffer);
    
    // Apply windowing and LP analysis
    use crate::codecs::g729a::tables::window_tables::{get_hamming_window, get_lag_window};
    use crate::codecs::g729a::math::dsp_operations::{autocorrelation, apply_lag_window};
    use crate::codecs::g729a::spectral::LinearPredictor;
    use crate::codecs::g729a::types::Q15;
    use crate::codecs::g729a::constants::LP_ORDER;
    
    let hamming_window = get_hamming_window();
    let lag_window = get_lag_window();
    
    // Apply Hamming window
    let mut windowed = Vec::new();
    for i in 0..240 {
        let sample = crate::codecs::g729a::math::fixed_point::mult(processed_samples[i].0, hamming_window[i].0);
        windowed.push(Q15(sample));
    }
    
    // Compute autocorrelation and apply lag window
    let mut correlations = autocorrelation(&windowed, LP_ORDER);
    apply_lag_window(&mut correlations, &lag_window);
    
    // Run Levinson-Durbin
    let predictor = LinearPredictor::new();
    let our_lp = predictor.analyze(&windowed);
    
    println!("Our LP coefficients (from signal): {:?}", 
        our_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Compare the differences
    println!("\n=== LP COEFFICIENT COMPARISON ===");
    let mut total_diff = 0i64;
    for i in 0..LP_ORDER {
        let diff = (our_lp.values[i].0 as i64) - (reference_lp.values[i].0 as i64);
        total_diff += diff.abs();
        println!("  LP[{}]: Our={:6}, Ref={:6}, Diff={:6}", 
            i, our_lp.values[i].0, reference_lp.values[i].0, diff);
    }
    
    println!("Total absolute difference: {}", total_diff);
    
    if total_diff < 1000 {
        println!("✅ LP coefficients are very close - quantization might be the issue");
    } else if total_diff < 10000 {
        println!("🔶 LP coefficients differ moderately - check windowing/preprocessing");
    } else {
        println!("❌ LP coefficients are very different - fundamental signal processing issue");
    }
    
    // Now check what LSP indices our LP coefficients would produce
    let our_lsp = converter.lp_to_lsp(&our_lp);
    
    use crate::codecs::g729a::spectral::LSPQuantizer;
    let mut quantizer = LSPQuantizer::new();
    let our_quantized = quantizer.quantize(&our_lsp);
    
    println!("\n=== LSP QUANTIZATION COMPARISON ===");
    println!("Our LP→LSP→Quantize: {:?}", our_quantized.indices);
    println!("ITU-T Reference:     {:?}", reference_indices);
    
    let mut index_matches = 0;
    for i in 0..4 {
        if our_quantized.indices[i] == reference_indices[i] {
            index_matches += 1;
            println!("  ✅ Index {}: {} matches", i, our_quantized.indices[i]);
        } else {
            println!("  ❌ Index {}: {} ≠ {}", i, our_quantized.indices[i], reference_indices[i]);
        }
    }
    
    if index_matches == 4 {
        println!("🎉 Perfect match! The issue is elsewhere in the encoder");
    } else if index_matches >= 2 {
        println!("🔶 Partial match - close but need fine-tuning");
    } else {
        println!("❌ No match - fundamental LP analysis or quantization issue");
    }
}

#[test]
fn test_frame0_signal_content() {
    println!("=== VERIFY FRAME 0 SIGNAL CONTENT ===");
    
    // Read ALGTHM.IN 
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    println!("Total samples in ALGTHM.IN: {}", all_samples.len());
    println!("Total frames: {}", all_samples.len() / 80);
    
    // Analyze first few frames
    for frame_idx in 0..4 {
        let start = frame_idx * 80;
        let end = start + 80;
        
        if end <= all_samples.len() {
            let frame_samples = &all_samples[start..end];
            
            let non_zero_count = frame_samples.iter().filter(|&&x| x != 0).count();
            let energy: i64 = frame_samples.iter().map(|&x| (x as i64) * (x as i64)).sum();
            let max_abs = frame_samples.iter().map(|&x| x.abs()).max().unwrap_or(0);
            let first_nonzero = frame_samples.iter().position(|&x| x != 0);
            
            println!("Frame {}: {} non-zero samples, energy={}, max_abs={}, first_nonzero={:?}",
                frame_idx, non_zero_count, energy, max_abs, first_nonzero);
            
            if non_zero_count > 0 {
                println!("  First 10 samples: {:?}", &frame_samples[0..10]);
                if let Some(pos) = first_nonzero {
                    let start_idx = pos.saturating_sub(2);
                    let end_idx = (pos + 3).min(frame_samples.len());
                    println!("  Around first non-zero [{}..{}]: {:?}", 
                        start_idx, end_idx, &frame_samples[start_idx..end_idx]);
                }
            }
        }
    }
    
    // **CRITICAL QUESTION**: Since Frame 0 is mostly silent, how does the ITU-T reference
    // produce meaningful LSP indices [16, 22, 22, 1]? 
    
    // **HYPOTHESIS 1**: G.729A uses a default/initialization LSP for silent frames
    // **HYPOTHESIS 2**: Our frame alignment is wrong - maybe Frame 0 isn't the first 80 samples
    // **HYPOTHESIS 3**: There's an initialization or state management issue
    
    println!("\n=== TESTING HYPOTHESIS: SILENT FRAME HANDLING ===");
    
    // Test what our LP analysis produces for a completely silent frame
    let silent_frame = vec![0i16; 240]; // 240-sample analysis window
    
    use crate::codecs::g729a::signal::Preprocessor;
    use crate::codecs::g729a::spectral::{LinearPredictor, LSPConverter, LSPQuantizer};
    use crate::codecs::g729a::types::Q15;
    
    let mut preprocessor = Preprocessor::new();
    let processed = preprocessor.process(&silent_frame);
    
    let predictor = LinearPredictor::new();
    let silent_lp = predictor.analyze(&processed);
    
    println!("LP coefficients for silent frame: {:?}", 
        silent_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Convert to LSP and quantize
    let converter = LSPConverter::new();
    let silent_lsp = converter.lp_to_lsp(&silent_lp);
    
    let mut quantizer = LSPQuantizer::new();
    let silent_quantized = quantizer.quantize(&silent_lsp);
    
    println!("Silent frame LSP indices: {:?}", silent_quantized.indices);
    
    // Compare with ITU-T reference
    let reference_indices = [16u8, 22u8, 22u8, 1u8];
    println!("ITU-T Frame 0 reference: {:?}", reference_indices);
    
    if silent_quantized.indices == reference_indices {
        println!("🎉 MATCH! ITU-T Frame 0 uses default quantization for silent frames");
    } else {
        println!("❌ No match - the issue is more complex");
        
        // Maybe we need to check the default initialization values
        println!("\n=== CHECKING DEFAULT/INITIALIZATION VALUES ===");
        
        // What does a fresh quantizer with no input produce?
        // (This might reveal the default state behavior)
    }
}

#[test]
fn test_debug_signal_placement_in_analysis_buffer() {
    println!("=== DEBUG SIGNAL PLACEMENT IN 240-SAMPLE ANALYSIS BUFFER ===");
    
    // Read ALGTHM.IN Frame 0
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    // Extract Frame 0 (samples 0-79) and Frame 1 lookahead (samples 80-119)
    println!("Frame 0 samples [0..80]: {:?}", &all_samples[0..10]);
    println!("Frame 0 samples [30..40]: {:?}", &all_samples[30..40]); // Where non-zero starts
    println!("Frame 1 lookahead [80..90]: {:?}", &all_samples[80..90]);
    
    // Build the 240-sample analysis buffer exactly like the encoder does
    let mut analysis_buffer = vec![0i16; 240];
    
    // History: For frame 0, use zeros (no previous frames) - positions 0-119
    // analysis_buffer[0..120] = zeros (already initialized)
    
    // Current frame: 80 samples starting at frame_idx * 80 - positions 120-199
    for i in 0..80 {
        if i < all_samples.len() {
            analysis_buffer[120 + i] = all_samples[i];
        }
    }
    
    // Lookahead: 40 samples - positions 200-239
    for i in 0..40 {
        if 80 + i < all_samples.len() {
            analysis_buffer[200 + i] = all_samples[80 + i];
        }
    }
    
    println!("\n=== ANALYSIS BUFFER CONTENT ===");
    println!("Total energy: {}", analysis_buffer.iter().map(|&x| (x as i64) * (x as i64)).sum::<i64>());
    println!("Non-zero count: {}", analysis_buffer.iter().filter(|&&x| x != 0).count());
    
    // Check where Frame 0's non-zero samples appear in the analysis buffer
    println!("Analysis buffer [115..125]: {:?}", &analysis_buffer[115..125]); // Around Frame 0 start
    println!("Analysis buffer [145..155]: {:?}", &analysis_buffer[145..155]); // Around sample 30 area
    println!("Analysis buffer [195..205]: {:?}", &analysis_buffer[195..205]); // Around Frame 0 end/lookahead start
    
    // Now check preprocessing step by step
    println!("\n=== PREPROCESSING DEBUG ===");
    
    use crate::codecs::g729a::signal::Preprocessor;
    let mut preprocessor = Preprocessor::new();
    let processed_samples = preprocessor.process(&analysis_buffer);
    
    println!("After preprocessing:");
    println!("Total energy: {}", processed_samples.iter().map(|&x| (x.0 as i64) * (x.0 as i64)).sum::<i64>());
    println!("Non-zero count: {}", processed_samples.iter().filter(|&&x| x.0 != 0).count());
    println!("Processed [115..125]: {:?}", processed_samples[115..125].iter().map(|x| x.0).collect::<Vec<_>>());
    println!("Processed [145..155]: {:?}", processed_samples[145..155].iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Check Hamming windowing
    println!("\n=== HAMMING WINDOWING DEBUG ===");
    
    use crate::codecs::g729a::tables::window_tables::get_hamming_window;
    use crate::codecs::g729a::math::fixed_point::mult;
    
    let hamming_window = get_hamming_window();
    
    // Apply Hamming window manually to see what happens
    let mut windowed = Vec::new();
    for i in 0..240 {
        let sample = mult(processed_samples[i].0, hamming_window[i].0);
        windowed.push(sample);
    }
    
    println!("After Hamming windowing:");
    println!("Total energy: {}", windowed.iter().map(|&x| (x as i64) * (x as i64)).sum::<i64>());
    println!("Non-zero count: {}", windowed.iter().filter(|&&x| x != 0).count());
    println!("Windowed [115..125]: {:?}", &windowed[115..125]);
    println!("Windowed [145..155]: {:?}", &windowed[145..155]);
    
    // Check where the maximum energy is in the windowed signal
    let mut max_energy = 0i64;
    let mut max_pos = 0;
    for i in 0..240 {
        let energy = (windowed[i] as i64) * (windowed[i] as i64);
        if energy > max_energy {
            max_energy = energy;
            max_pos = i;
        }
    }
    
    println!("Maximum energy {} at position {}", max_energy, max_pos);
    println!("Window values around max pos [{}..{}]: {:?}", 
        max_pos.saturating_sub(5), max_pos + 5,
        &hamming_window[max_pos.saturating_sub(5)..max_pos + 5].iter().map(|x| x.0).collect::<Vec<_>>());
    
    // **KEY INSIGHT CHECK**: The Hamming window has its peak around sample 120 (center of 240 samples)
    // If Frame 0's signal starts at sample 30 (relative), it appears at position 150 in our buffer
    // The Hamming window value at position 150 should be reasonably high, not zero
    
    println!("\n=== WINDOW VALUE ANALYSIS ===");
    println!("Window center (120): {}", hamming_window[120].0);
    println!("Window at pos 150 (Frame 0 signal area): {}", hamming_window[150].0);
    println!("Window at pos 30 (where signal would be if no offset): {}", hamming_window[30].0);
    
    // Check if our window implementation is wrong
    println!("First 10 window values: {:?}", hamming_window[0..10].iter().map(|x| x.0).collect::<Vec<_>>());
    println!("Window values [115..125]: {:?}", hamming_window[115..125].iter().map(|x| x.0).collect::<Vec<_>>());
    println!("Window values [145..155]: {:?}", hamming_window[145..155].iter().map(|x| x.0).collect::<Vec<_>>());
    
    // **HYPOTHESIS**: If preprocessing or windowing is zeroing out our signal,
    // the issue might be in:
    // 1. Preprocessor high-pass filter removing the signal
    // 2. Wrong window coefficients
    // 3. Signal placement in wrong buffer positions
    
    println!("\n=== CONCLUSION ===");
    if windowed.iter().any(|&x| x.abs() > 1000) {
        println!("✅ Windowed signal has significant values - issue might be elsewhere");
    } else {
        println!("❌ Windowed signal is too small - preprocessing or windowing issue confirmed");
    }
}

#[test]
fn test_bypass_preprocessing_to_fix_compliance() {
    println!("=== TEST: BYPASS PREPROCESSING TO FIX COMPLIANCE ===");
    
    // Read ALGTHM.IN Frame 0
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    // Build analysis buffer with Frame 0 signal
    let mut analysis_buffer = vec![0i16; 240];
    for i in 0..80 {
        if i < all_samples.len() {
            analysis_buffer[120 + i] = all_samples[i];
        }
    }
    for i in 0..40 {
        if 80 + i < all_samples.len() {
            analysis_buffer[200 + i] = all_samples[80 + i];
        }
    }
    
    println!("Raw analysis buffer energy: {}", 
        analysis_buffer.iter().map(|&x| (x as i64) * (x as i64)).sum::<i64>());
    
    // **TEST 1: Skip preprocessing entirely**
    println!("\n=== TEST 1: NO PREPROCESSING ===");
    
    use crate::codecs::g729a::types::Q15;
    
    // Convert raw samples directly to Q15
    let raw_q15: Vec<Q15> = analysis_buffer.iter().map(|&x| Q15(x)).collect();
    
    use crate::codecs::g729a::spectral::{LinearPredictor, LSPConverter, LSPQuantizer};
    
    let predictor = LinearPredictor::new();
    let raw_lp = predictor.analyze(&raw_q15);
    
    println!("Raw LP coefficients: {:?}", 
        raw_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    let converter = LSPConverter::new();
    let raw_lsp = converter.lp_to_lsp(&raw_lp);
    
    let mut quantizer = LSPQuantizer::new();
    let raw_quantized = quantizer.quantize(&raw_lsp);
    
    println!("Raw signal LSP indices: {:?}", raw_quantized.indices);
    
    // **TEST 2: Only apply pre-emphasis (skip high-pass filter)**
    println!("\n=== TEST 2: PRE-EMPHASIS ONLY ===");
    
    // Apply only pre-emphasis filter manually
    const PREEMPH_FACTOR: i16 = 22282; // 0.68 * 32768
    let mut preemph_mem = 0i16;
    let mut preemph_only = Vec::new();
    
    for &sample in &analysis_buffer {
        let preemph_contrib = ((preemph_mem as i32 * PREEMPH_FACTOR as i32) + 16384) >> 15;
        let output = sample as i32 - preemph_contrib;
        let saturated = output.clamp(-32768, 32767) as i16;
        preemph_only.push(Q15(saturated));
        preemph_mem = sample;
    }
    
    println!("Pre-emphasis only energy: {}", 
        preemph_only.iter().map(|&x| (x.0 as i64) * (x.0 as i64)).sum::<i64>());
    
    let preemph_lp = predictor.analyze(&preemph_only);
    let preemph_lsp = converter.lp_to_lsp(&preemph_lp);
    
    let mut quantizer2 = LSPQuantizer::new();
    let preemph_quantized = quantizer2.quantize(&preemph_lsp);
    
    println!("Pre-emphasis only LSP indices: {:?}", preemph_quantized.indices);
    
    // **TEST 3: Compare with ITU-T reference**
    let reference_indices = [16u8, 22u8, 22u8, 1u8];
    println!("\nITU-T reference indices: {:?}", reference_indices);
    
    println!("\n=== COMPARISON RESULTS ===");
    
    // Check which approach matches best
    let raw_matches = raw_quantized.indices.iter().zip(reference_indices.iter())
        .filter(|(a, b)| a == b).count();
    let preemph_matches = preemph_quantized.indices.iter().zip(reference_indices.iter())
        .filter(|(a, b)| a == b).count();
    
    println!("Raw signal matches: {}/4", raw_matches);
    println!("Pre-emphasis only matches: {}/4", preemph_matches);
    
    if raw_matches > preemph_matches {
        println!("🎯 SOLUTION: Skip preprocessing entirely!");
    } else if preemph_matches > raw_matches {
        println!("🎯 SOLUTION: Use only pre-emphasis, skip high-pass filter!");
    } else if raw_matches >= 2 || preemph_matches >= 2 {
        println!("🔶 PROGRESS: Found better approach, need fine-tuning");
    } else {
        println!("❌ Neither approach works - deeper issue remains");
    }
    
    // **HYPOTHESIS**: The ITU-T G.729A reference might use different preprocessing
    // or our filter coefficients might be wrong. The high-pass filter is removing
    // too much energy from the signal, making LP analysis produce wrong coefficients.
}

#[test]
fn test_decode_reference_indices_to_find_expected_signal() {
    println!("=== DECODE ITU-T REFERENCE INDICES TO EXPECTED SIGNAL ===");
    
    // Decode the CORRECT ITU-T Frame 0 LSP indices [1, 105, 17, 0]
    let reference_indices = [1u8, 105u8, 17u8, 0u8];
    
    use crate::codecs::g729a::spectral::{LSPDecoder, LSPConverter};
    let mut decoder = LSPDecoder::new();
    let reference_lsp = decoder.decode(&reference_indices);
    
    println!("ITU-T Reference LSP indices: {:?}", reference_indices);
    println!("ITU-T Reference LSP frequencies: {:?}", 
        reference_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Convert reference LSP to LP coefficients
    let converter = LSPConverter::new();
    let reference_lp = converter.lsp_to_lp(&reference_lsp);
    
    println!("ITU-T Reference LP coefficients: {:?}", 
        reference_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Compare with what our bcg729-exact algorithms produce
    println!("\n=== COMPARE WITH OUR bcg729-EXACT ALGORITHMS ===");
    
    // Read ALGTHM.IN Frame 0 and process with bcg729-exact chain
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.IN")
        .expect("Failed to open ALGTHM.IN");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.IN");
    
    let mut all_samples = Vec::new();
    for i in 0..buffer.len()/2 {
        let sample_idx = i * 2;
        if sample_idx + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[sample_idx], buffer[sample_idx + 1]]);
            all_samples.push(sample);
        }
    }
    
    // Build Frame 0 analysis buffer exactly like the encoder
    let mut analysis_buffer = vec![Q15(0); 240];
    
    // Current frame: Frame 0 (samples 0-79) at positions 120-199
    for i in 0..80 {
        if i < all_samples.len() {
            analysis_buffer[120 + i] = Q15(all_samples[i]);
        }
    }
    
    // Lookahead: Frame 1 first 40 samples (samples 80-119) at positions 200-239
    for i in 0..40 {
        if 80 + i < all_samples.len() {
            analysis_buffer[200 + i] = Q15(all_samples[80 + i]);
        }
    }
    
    println!("Frame 0 analysis buffer energy: {}", 
        analysis_buffer.iter().map(|&x| (x.0 as i64) * (x.0 as i64)).sum::<i64>());
    
    // Apply bcg729-exact processing 
    use crate::codecs::g729a::spectral::LinearPredictor;
    let predictor = LinearPredictor::new();
    let our_lp = predictor.analyze(&analysis_buffer);
    
    println!("Our bcg729-exact LP coefficients: {:?}", 
        our_lp.values.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Calculate the difference
    println!("\n=== LP COEFFICIENT ANALYSIS ===");
    let mut total_difference = 0i64;
    for i in 0..10 {
        let diff = (our_lp.values[i].0 as i64) - (reference_lp.values[i].0 as i64);
        total_difference += diff.abs();
        println!("  LP[{}]: Our={:6}, ITU-T={:6}, Diff={:6}", 
            i, our_lp.values[i].0, reference_lp.values[i].0, diff);
    }
    
    println!("Total absolute LP difference: {}", total_difference);
    
    if total_difference < 10000 {
        println!("🎉 LP coefficients are very close - signal processing is correct!");
    } else if total_difference < 100000 {
        println!("🔶 LP coefficients are somewhat close - minor algorithm differences");
    } else {
        println!("❌ LP coefficients are very different - major signal or algorithm issue");
        
        // **KEY INSIGHT**: If our LP coefficients are very different, it could mean:
        // 1. We're analyzing the wrong Frame 0 content
        // 2. The ITU-T reference uses different signal preprocessing
        // 3. The ALGTHM.IN Frame 0 is not the same as what produced the reference LSP indices
        
        println!("\n🔍 DIAGNOSTIC: This suggests Frame 0 in ALGTHM.IN may not be the exact");
        println!("   signal that was used to generate the reference LSP indices [16,22,22,1].");
        println!("   The ITU-T test vectors might be synthetic or from a different source.");
    }
    
    // Also test our quantization of the correct reference LSP
    println!("\n=== TEST QUANTIZATION OF REFERENCE LSP ===");
    
    use crate::codecs::g729a::spectral::LSPQuantizer;
    let mut quantizer = LSPQuantizer::new();
    let quantized_reference = quantizer.quantize(&reference_lsp);
    
    println!("Reference LSP quantized: {:?}", quantized_reference.indices);
    println!("Expected indices:        {:?}", reference_indices);
    
    let ref_matches = quantized_reference.indices.iter().zip(reference_indices.iter())
        .filter(|(a, b)| a == b).count();
    
    println!("Reference self-consistency: {}/4 matches", ref_matches);
    
    if ref_matches == 4 {
        println!("✅ Our quantizer is self-consistent with reference LSP");
    } else {
        println!("❌ Our quantizer produces different results for reference LSP");
        println!("   This indicates a fundamental quantizer algorithm issue");
    }
}

#[test]
fn test_bcg729_exact_quantizer_self_consistency() {
    println!("=== TEST BCG729-EXACT QUANTIZER SELF-CONSISTENCY ===");
    
    // Decode the CORRECT ITU-T Frame 0 LSP indices [1, 105, 17, 0]
    let reference_indices = [1u8, 105u8, 17u8, 0u8];
    
    use crate::codecs::g729a::spectral::{LSPDecoder, LSPQuantizer};
    
    // Step 1: Decode reference indices to get LSP
    let mut decoder = LSPDecoder::new();
    let reference_lsp = decoder.decode(&reference_indices);
    
    println!("ITU-T Reference indices: {:?}", reference_indices);
    println!("Decoded LSP frequencies: {:?}", 
        reference_lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
    
    // Step 2: Re-quantize the same LSP
    let mut quantizer = LSPQuantizer::new();
    let quantized_result = quantizer.quantize(&reference_lsp);
    
    println!("\n=== SELF-CONSISTENCY TEST ===");
    println!("Re-quantized indices: {:?}", quantized_result.indices);
    println!("Expected indices:     {:?}", reference_indices);
    
    let mut matches = 0;
    for i in 0..4 {
        if quantized_result.indices[i] == reference_indices[i] {
            matches += 1;
            println!("  ✅ Index {}: {} matches", i, quantized_result.indices[i]);
        } else {
            println!("  ❌ Index {}: {} ≠ {}", i, quantized_result.indices[i], reference_indices[i]);
        }
    }
    
    println!("\nSelf-consistency: {}/4 matches", matches);
    
    if matches == 4 {
        println!("🎉 QUANTIZER PASSES SELF-CONSISTENCY TEST!");
        println!("   This proves our bcg729-exact implementation is correct!");
    } else {
        println!("❌ Quantizer still has issues - needs more debugging");
        
        // Debug: decode our result and compare LSP values
        let our_decoded = decoder.decode(&quantized_result.indices);
        println!("\nDebug LSP comparison:");
        for i in 0..10 {
            let ref_val = reference_lsp.frequencies[i].0;
            let our_val = our_decoded.frequencies[i].0;
            let diff = (ref_val as i32 - our_val as i32).abs();
            println!("  LSP[{}]: ref={:6}, our={:6}, diff={:6}", i, ref_val, our_val, diff);
        }
    }
    
    assert_eq!(matches, 4, "Quantizer must pass self-consistency test");
}

#[test]
fn test_debug_lsp_lsf_conversion() {
    println!("=== DEBUG LSP-LSF CONVERSION ===");
    
    use crate::codecs::g729a::spectral::quantizer::{g729_acos_q15q13, g729_cos_q13q15};
    
    // Test with known values
    let test_values = [32767, 16384, 0, -16384, -32768];
    
    println!("Testing arccos (LSP Q15 -> LSF Q13):");
    for &val in &test_values {
        let lsf = g729_acos_q15q13(val);
        println!("  LSP {} -> LSF {}", val, lsf);
    }
    
    println!("\nTesting cosine (LSF Q13 -> LSP Q15):");
    let test_lsf = [0, 6434, 12868, 19302, 25736];
    for &val in &test_lsf {
        let lsp = g729_cos_q13q15(val);
        println!("  LSF {} -> LSP {}", val, lsp);
    }
    
    // Test round-trip
    println!("\nRound-trip test:");
    let original_lsp = [5000i16, 10000, 15000, 20000, -5000];
    for &val in &original_lsp {
        let lsf = g729_acos_q15q13(val);
        let back_to_lsp = g729_cos_q13q15(lsf);
        println!("  LSP {} -> LSF {} -> LSP {} (diff: {})", 
            val, lsf, back_to_lsp, (val - back_to_lsp).abs());
    }
}

#[test]
fn test_trace_reference_decode() {
    println!("=== TRACE REFERENCE DECODE [16, 22, 22, 1] ===");
    
    use crate::codecs::g729a::spectral::LSPDecoder;
    use crate::codecs::g729a::tables::{LSP_CB1, LSP_CB2};
    
    let indices = [16u8, 22u8, 22u8, 1u8];
    let l0 = indices[0] as usize;
    let l1 = indices[1] as usize; 
    let l2 = indices[2] as usize;
    let l3 = indices[3] as usize;
    
    println!("Indices: L0={}, L1={}, L2={}, L3={}", l0, l1, l2, l3);
    
    // Check codebook values
    println!("\nCodebook L1[{}]:", l1);
    if l1 < LSP_CB1.len() {
        println!("  Values: {:?}", LSP_CB1[l1]);
    }
    
    println!("\nCodebook L2[{}] (first 5):", l2);
    if l2 < LSP_CB2.len() {
        println!("  Values: {:?}", &LSP_CB2[l2][0..5]);
    }
    
    println!("\nCodebook L3[{}] (last 5):", l3);
    if l3 < LSP_CB2.len() {
        println!("  Values: {:?}", &LSP_CB2[l3][5..10]);
    }
    
    // Reconstruct quantizer output
    let mut quantizer_output = [0i16; 10];
    for i in 0..5 {
        quantizer_output[i] = LSP_CB1[l1][i] + LSP_CB2[l2][i];
        println!("quantizer_output[{}] = {} + {} = {}", 
            i, LSP_CB1[l1][i], LSP_CB2[l2][i], quantizer_output[i]);
    }
    for i in 5..10 {
        quantizer_output[i] = LSP_CB1[l1][i] + LSP_CB2[l3][i];
        println!("quantizer_output[{}] = {} + {} = {}", 
            i, LSP_CB1[l1][i], LSP_CB2[l3][i], quantizer_output[i]);
    }
    
    println!("\nQuantizer output before rearrange: {:?}", quantizer_output);
    
    // Now decode with our decoder
    let mut decoder = LSPDecoder::new();
    let decoded = decoder.decode(&indices);
    
    println!("\nFinal decoded LSP: {:?}", 
        decoded.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
}

#[test]
fn test_parse_algthm_bit_frame0_manually() {
    println!("=== MANUALLY PARSE ALGTHM.BIT FRAME 0 ===");
    
    use std::fs::File;
    use std::io::Read;
    
    // Read ALGTHM.BIT in ITU-T expanded format
    let mut file = File::open("src/codecs/g729a/tests/test_vectors/ALGTHM.BIT")
        .expect("Failed to open ALGTHM.BIT");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read ALGTHM.BIT");
    
    // ITU-T expanded format: each bit is 2 bytes (0x007F='0', 0x0081='1')
    // Skip sync word (2 bytes) and frame size (2 bytes)
    let mut bit_pos = 4;
    
    println!("First few bytes: {:02X?}", &buffer[0..20]);
    
    // Frame 0 starts after sync/size header
    // G.729A frame = 80 bits = 160 bytes in expanded format
    let frame_start = bit_pos;
    let frame_end = frame_start + 160;
    
    if frame_end > buffer.len() {
        println!("ERROR: File too short for frame 0");
        return;
    }
    
    // Extract bits for Frame 0
    let mut frame_bits = Vec::new();
    for i in (frame_start..frame_end).step_by(2) {
        let bit_value = u16::from_le_bytes([buffer[i], buffer[i+1]]);
        let bit = if bit_value == 0x0081 { 1u8 } else { 0u8 };
        frame_bits.push(bit);
    }
    
    println!("Frame 0 bits (80 total): {:?}", &frame_bits[0..20]);
    
    // Parse LSP parameters according to G.729A bit allocation
    // L0: 1 bit
    // L1: 7 bits  
    // L2: 5 bits
    // L3: 5 bits
    let l0 = frame_bits[0];
    
    let mut l1 = 0u8;
    for i in 0..7 {
        l1 = (l1 << 1) | frame_bits[1 + i];
    }
    
    let mut l2 = 0u8;
    for i in 0..5 {
        l2 = (l2 << 1) | frame_bits[8 + i];
    }
    
    let mut l3 = 0u8;
    for i in 0..5 {
        l3 = (l3 << 1) | frame_bits[13 + i];
    }
    
    println!("\n=== PARSED LSP INDICES ===");
    println!("L0: {} (predictor select)", l0);
    println!("L1: {} (stage 1 VQ)", l1);
    println!("L2: {} (stage 2 lower)", l2);
    println!("L3: {} (stage 2 upper)", l3);
    
    println!("\nAs array: [{}, {}, {}, {}]", l0, l1, l2, l3);
    
    // Compare with what we thought were the indices
    println!("\nWe thought indices were: [16, 22, 22, 1]");
    println!("Actual parsed indices:   [{}, {}, {}, {}]", l0, l1, l2, l3);
}

#[test]
fn test_debug_vq_search_l1() {
    println!("=== DEBUG VQ SEARCH FOR L1 (7 vs 105) ===");
    
    use crate::codecs::g729a::spectral::{LSPDecoder, LSPQuantizer};
    use crate::codecs::g729a::tables::{LSP_CB1};
    
    // First decode the correct reference to get target LSF
    let reference_indices = [1u8, 105u8, 17u8, 0u8];
    let mut decoder = LSPDecoder::new();
    let reference_lsp = decoder.decode(&reference_indices);
    
    println!("Reference LSP frequencies: {:?}", 
        reference_lsp.frequencies.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
    
    // Now let's trace through why our quantizer picks L1=7 instead of L1=105
    let mut quantizer = LSPQuantizer::new();
    
    // Convert LSP to LSF to see the target
    use crate::codecs::g729a::spectral::quantizer::g729_acos_q15q13;
    let mut target_lsf = [0i16; 10];
    for i in 0..10 {
        target_lsf[i] = g729_acos_q15q13(reference_lsp.frequencies[i].0);
    }
    
    println!("\nTarget LSF (after arccos): {:?}", &target_lsf[0..5]);
    
    // Check what's in codebook entries 7 and 105
    println!("\nCodebook comparison:");
    println!("L1[7]:   {:?}", &LSP_CB1[7][0..5]);
    println!("L1[105]: {:?}", &LSP_CB1[105][0..5]);
    
    // Calculate distances manually
    let mut dist_7 = 0i64;
    let mut dist_105 = 0i64;
    
    println!("\nPer-coefficient analysis:");
    for i in 0..10 {
        let diff_7 = (target_lsf[i] as i64) - (LSP_CB1[7][i] as i64);
        let diff_105 = (target_lsf[i] as i64) - (LSP_CB1[105][i] as i64);
        
        dist_7 += diff_7 * diff_7;
        dist_105 += diff_105 * diff_105;
        
        if i < 5 {
            println!("  Coeff[{}]: target={}, L1[7]={}, L1[105]={}, diff_7={}, diff_105={}", 
                i, target_lsf[i], LSP_CB1[7][i], LSP_CB1[105][i], diff_7.abs(), diff_105.abs());
        }
    }
    
    println!("\nTotal squared distances:");
    println!("  L1[7]:   {}", dist_7);
    println!("  L1[105]: {}", dist_105);
    
    if dist_7 < dist_105 {
        println!("\n❌ Our simple distance favors L1[7] - need to check MA prediction!");
    } else {
        println!("\n✅ L1[105] should win based on distance");
    }
    
    // The issue might be in the MA prediction and target vector computation
    println!("\n=== CHECKING MA PREDICTION ===");
    
    // Manually compute what the target vector should be for L0=1
    // This is where the bug likely is
}

#[test]
fn test_trace_decoder_ma_prediction() {
    println!("=== TRACE DECODER MA PREDICTION ===");
    
    use crate::codecs::g729a::tables::{LSP_CB1, LSP_CB2};
    
    let indices = [1u8, 105u8, 17u8, 0u8];
    
    // Step 1: Reconstruct quantizer output from codebooks
    let mut quantizer_output = [0i16; 10];
    for i in 0..5 {
        quantizer_output[i] = LSP_CB1[105][i] + LSP_CB2[17][i];
    }
    for i in 5..10 {
        quantizer_output[i] = LSP_CB1[105][i] + LSP_CB2[0][i];
    }
    
    println!("Quantizer output (before rearrange): {:?}", quantizer_output);
    
    // Step 2: Apply rearrangements (should be minimal for correct values)
    // Skip for now to see raw values
    
    // Step 3: The key issue - MA prediction for L0=1
    // The decoder should compute: qLSF = MAPredictorSum * quantizer_output + MA_prediction
    
    // Check what MA predictor coefficients we're using for L0=1
    println!("\n=== MA PREDICTOR DEBUG ===");
    
    // The decoder initialization should have the correct previous LSF values
    // bcg729 initializes with: [2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396]
    let initial_lsf = [2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396];
    println!("Initial prev_lsf: {:?}", &initial_lsf[0..5]);
    
    // For first frame, all 4 previous frames have the same initial values
    // So the MA prediction should be based on these initial values
    
    // The issue is likely that we're not applying the MA prediction correctly
    // or the MA predictor coefficients are wrong
}

#[test]
fn test_debug_target_vector_computation() {
    println!("=== DEBUG TARGET VECTOR COMPUTATION ===");
    
    // Reference indices [1, 105, 17, 0] where L0=1
    let l0 = 1;
    
    // Decode to get the expected LSF values
    use crate::codecs::g729a::spectral::LSPDecoder;
    let mut decoder = LSPDecoder::new();
    let reference_indices = [1u8, 105u8, 17u8, 0u8];
    let reference_lsp = decoder.decode(&reference_indices);
    
    // Convert to LSF
    use crate::codecs::g729a::spectral::quantizer::g729_acos_q15q13;
    let mut lsf = [0i16; 10];
    for i in 0..10 {
        lsf[i] = g729_acos_q15q13(reference_lsp.frequencies[i].0);
    }
    
    println!("LSF values: {:?}", &lsf[0..5]);
    
    // Initial prev_lsf should be [2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396]
    let prev_lsf = [2339i16, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396];
    
    // MA predictor for L0=1, frame 0
    let ma_pred = [7733i16, 7880, 8188, 8175, 8247, 8490, 8637, 8601, 8359, 7569];
    
    // Compute target vector
    println!("\nTarget vector computation for first 3 coefficients:");
    for i in 0..3 {
        let mut acc = (lsf[i] as i32) << 15; // Q13 -> Q28
        println!("  Coeff[{}]: LSF={} -> acc_init={}", i, lsf[i], acc);
        
        // For first frame, all 4 previous frames have same values
        for j in 0..4 {
            let ma_val = (prev_lsf[i] as i32) * (ma_pred[i] as i32);
            acc -= ma_val;
            println!("    Frame[{}]: prev_lsf={} * ma_pred={} = {} -> acc={}", 
                j, prev_lsf[i], ma_pred[i], ma_val, acc);
        }
        
        // Convert back to Q13
        let temp_q13 = (acc >> 15) as i16;
        println!("    -> temp_q13={}", temp_q13);
        
        // Apply inverse MA predictor sum (Q12)
        // For L0=1, coeff 0: inv_sum = 9202
        let inv_sums = [9202i16, 7320, 6788, 7738, 8170, 8154, 8856, 8818, 8366, 8544];
        let target = ((temp_q13 as i32 * inv_sums[i] as i32) >> 12) as i16;
        println!("    -> target_vector[{}] = {} * {} >> 12 = {}", i, temp_q13, inv_sums[i], target);
    }
}

#[test]
fn test_force_correct_lsp_to_quantizer() {
    println!("=== FORCE CORRECT LSP TO QUANTIZER ===");
    
    // From our reverse engineering, the LSF that should produce [1, 105, 17, 0] is:
    // [2254, 3389, 4623, 7659, 9837, ...]
    
    // Convert these LSF values to LSP
    use crate::codecs::g729a::spectral::quantizer::g729_cos_q13q15;
    let target_lsf = [2254i16, 3389, 4623, 7659, 9837, 12500, 15000, 17500, 20000, 22500];
    
    let mut lsp_frequencies = [crate::codecs::g729a::types::Q15::ZERO; 10];
    for i in 0..10 {
        lsp_frequencies[i] = crate::codecs::g729a::types::Q15(g729_cos_q13q15(target_lsf[i]));
    }
    
    println!("Target LSF: {:?}", &target_lsf[0..5]);
    println!("Converted LSP: {:?}", lsp_frequencies.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
    
    // Now quantize these LSP values
    use crate::codecs::g729a::spectral::LSPQuantizer;
    use crate::codecs::g729a::types::LSPParameters;
    let mut quantizer = LSPQuantizer::new();
    let lsp_params = LSPParameters { frequencies: lsp_frequencies };
    let quantized = quantizer.quantize(&lsp_params);
    
    println!("\nQuantized indices: {:?}", quantized.indices);
    println!("Expected indices:  [1, 105, 17, 0]");
    
    let mut matches = 0;
    for i in 0..4 {
        if quantized.indices[i] == [1u8, 105, 17, 0][i] {
            matches += 1;
            println!("  ✅ Index {}: {}", i, quantized.indices[i]);
        } else {
            println!("  ❌ Index {}: {} ≠ {}", i, quantized.indices[i], [1, 105, 17, 0][i]);
        }
    }
    
    println!("\nMatches: {}/4", matches);
    
    if matches == 4 {
        println!("✅ Quantizer WORKS when given the right LSP!");
        println!("   This proves the quantizer algorithm is correct.");
        println!("   The issue is in LP analysis or LSP conversion.");
    } else {
        println!("❌ Quantizer still has issues even with perfect input");
    }
}

#[test]
fn test_debug_exact_arccos() {
    println!("=== DEBUG EXACT ARCCOS ===");
    
    use crate::codecs::g729a::spectral::quantizer::g729_acos_q15q13;
    
    // Test some key values
    let test_values = [
        (32767, "1.0"),      // cos(0) = 1
        (0, "0.0"),          // cos(π/2) = 0
        (-32767, "-1.0"),    // cos(π) = -1
        (23170, "0.707"),    // cos(π/4) ≈ 0.707
        (31535, "0.962"),    // From our test case
    ];
    
    for (val, desc) in test_values.iter() {
        let result = g729_acos_q15q13(*val);
        println!("acos({} = {}) = {} (expected range: 0-25736)", val, desc, result);
    }
    
    // Also test the intermediate functions
    println!("\nTest sqrt function:");
    use crate::codecs::g729a::spectral::quantizer::g729_sqrt_q0q7;
    let sqrt_test = g729_sqrt_q0q7(1073741824); // 1.0 in Q30
    println!("sqrt(1.0 in Q30) = {} (expected ~128 in Q7)", sqrt_test);
    
    // Test with smaller value that should work
    let sqrt_1_q0 = g729_sqrt_q0q7(1); // 1 in Q0
    println!("sqrt(1 in Q0) = {} (expected ~128 in Q7)", sqrt_1_q0);
    
    // Test with 16384 (0.25 in Q16 shifted to Q0)
    let sqrt_025 = g729_sqrt_q0q7(16384); // 0.25 when considered as Q16->Q0
    println!("sqrt(16384) = {} (expected ~64 in Q7 for sqrt(0.5))", sqrt_025);
}

#[test]
fn test_debug_l1_codebook_entry_105() {
    println!("=== DEBUG L1 CODEBOOK ENTRY 105 ===");
    
    use crate::codecs::g729a::spectral::LSPQuantizer;
    let quantizer = LSPQuantizer::new();
    
    // Print codebook entry 105
    println!("L1 codebook entry 105:");
    for i in 0..10 {
        println!("  CB1[105][{}] = {}", i, quantizer.get_l1_codebook(105, i));
    }
    
    // Expected target vector from our test
    let target_vector = [2143i16, 2376, 3044, 6152, 7980, 10000, 12000, 14000, 16000, 18000];
    
    // Calculate distance to entry 105
    let mut dist_105 = 0i64;
    for i in 0..10 {
        let diff = (target_vector[i] as i32) - (quantizer.get_l1_codebook(105, i) as i32);
        dist_105 += (diff as i64) * (diff as i64);
    }
    println!("\nDistance to entry 105: {}", dist_105);
    
    // Also check entry 7 which was selected
    println!("\nL1 codebook entry 7:");
    for i in 0..10 {
        println!("  CB1[7][{}] = {}", i, quantizer.get_l1_codebook(7, i));
    }
    
    let mut dist_7 = 0i64;
    for i in 0..10 {
        let diff = (target_vector[i] as i32) - (quantizer.get_l1_codebook(7, i) as i32);
        dist_7 += (diff as i64) * (diff as i64);
    }
    println!("\nDistance to entry 7: {}", dist_7);
    
    println!("\nEntry 7 distance ({}) < Entry 105 distance ({}): {}", 
             dist_7, dist_105, dist_7 < dist_105);
}