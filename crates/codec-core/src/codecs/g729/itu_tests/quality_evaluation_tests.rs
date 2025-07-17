//! G.729 Quality Evaluation Tests
//!
//! This module contains tests that evaluate the actual quality and performance
//! of G.729 codec components against ITU-T reference implementations.
//!
//! Unlike basic functionality tests, these tests measure:
//! - ACELP search quality (pulse distribution, track usage)
//! - LSP quantization accuracy (distortion metrics)  
//! - Pitch analysis accuracy (correlation quality, lag estimation)
//! - Synthesis filter quality (signal characteristics)
//! - Real ITU compliance (bitstream and audio comparison)

#[allow(missing_docs)]

use crate::codecs::g729::src::encoder::G729Encoder;
use crate::codecs::g729::src::decoder::G729Decoder;
use crate::codecs::g729::src::types::*;
use crate::codecs::g729::src::lpc::LpcAnalyzer;
use crate::codecs::g729::src::pitch::PitchAnalyzer;
use crate::codecs::g729::src::acelp::AcelpAnalyzer;
use crate::codecs::g729::src::quantization::{LspQuantizer, GainQuantizer};
use super::itu_test_utils::*;

/// Test ACELP search quality and pulse distribution
#[test]
fn test_acelp_search_quality() {
    println!("🎯 ACELP Search Quality Evaluation");
    
    let mut encoder = G729Encoder::new();
    let mut quality_metrics = AcelpQualityMetrics::new();
    
    // Test with various signal types to evaluate ACELP performance
    let test_signals = generate_test_signals();
    
    for (signal_name, signal) in &test_signals {
        println!("  Testing ACELP with: {}", signal_name);
        
        let mut pulse_position_stats = PulsePositionStats::new();
        let mut track_usage_stats = TrackUsageStats::new();
        
        // Process multiple frames to get statistics
        for frame_chunk in signal.chunks(80) {
            if frame_chunk.len() == 80 {
                let g729_frame = encoder.encode_frame(frame_chunk);
                
                // Analyze each subframe's ACELP parameters
                for (subframe_idx, subframe) in g729_frame.subframes.iter().enumerate() {
                    pulse_position_stats.add_frame(&subframe.positions);
                    track_usage_stats.add_frame(&subframe.positions);
                    
                    println!("    Subframe {}: positions={:?}, tracks_used={}", 
                             subframe_idx, subframe.positions, 
                             count_unique_tracks(&subframe.positions));
                }
            }
        }
        
        // Calculate quality metrics
        let position_diversity = pulse_position_stats.calculate_diversity();
        let track_utilization = track_usage_stats.calculate_utilization();
        let clustering_penalty = pulse_position_stats.calculate_clustering_penalty();
        
        println!("    📊 ACELP Quality Metrics:");
        println!("      Position diversity: {:.1}%", position_diversity * 100.0);
        println!("      Track utilization: {:.1}%", track_utilization * 100.0);
        println!("      Clustering penalty: {:.1}%", clustering_penalty * 100.0);
        
        // Store results for overall assessment
        quality_metrics.add_result(signal_name.clone(), AcelpResult {
            position_diversity,
            track_utilization,
            clustering_penalty,
        });
    }
    
    // Overall ACELP quality assessment
    let overall_quality = quality_metrics.calculate_overall_score();
    println!("  🎯 Overall ACELP Quality: {:.1}%", overall_quality * 100.0);
    
    // Quality thresholds for assessment
    if overall_quality > 0.8 {
        println!("  ✅ EXCELLENT - ACELP search is working well");
    } else if overall_quality > 0.6 {
        println!("  🟡 GOOD - ACELP search has minor issues");
    } else if overall_quality > 0.3 {
        println!("  🟠 NEEDS IMPROVEMENT - ACELP search has significant issues");
    } else {
        println!("  ❌ POOR - ACELP search needs major optimization");
        println!("      Recommended fixes:");
        println!("      - Implement proper correlation matrix H^T*H computation");
        println!("      - Add track-constrained search algorithm");
        println!("      - Implement iterative pulse refinement");
    }
}

/// Test LSP quantization quality and distortion
#[test]
fn test_lsp_quantization_quality() {
    println!("🎯 LSP Quantization Quality Evaluation");
    
    let mut lsp_quantizer = LspQuantizer::new();
    let mut quality_metrics = LspQualityMetrics::new();
    
    // Generate test LSP vectors across the valid range
    let test_lsp_vectors = generate_test_lsp_vectors();
    
    for (vector_name, lsp_vector) in &test_lsp_vectors {
        println!("  Testing LSP quantization: {}", vector_name);
        
        // Quantize and dequantize
        let mut lsp_quantized = [0i16; M];
        let indices = lsp_quantizer.quantize_lsp(lsp_vector, &mut lsp_quantized);
        
        // Calculate distortion metrics
        let spectral_distortion = calculate_spectral_distortion(lsp_vector, &lsp_quantized);
        let lsf_distortion = calculate_lsf_distortion(lsp_vector, &lsp_quantized);
        let ordering_violations = check_lsp_ordering(&lsp_quantized);
        let stability_margin = calculate_stability_margin(&lsp_quantized);
        
        println!("    📊 LSP Quality Metrics:");
        println!("      Spectral distortion: {:.2} dB", spectral_distortion);
        println!("      LSF distortion: {:.4}", lsf_distortion);
        println!("      Ordering violations: {}", ordering_violations);
        println!("      Stability margin: {:.4}", stability_margin);
        println!("      Quantization indices: {:?}", indices);
        
        quality_metrics.add_result(vector_name.clone(), LspResult {
            spectral_distortion,
            lsf_distortion,
            ordering_violations,
            stability_margin,
        });
    }
    
    let overall_quality = quality_metrics.calculate_overall_score();
    println!("  🎯 Overall LSP Quality: {:.1}%", overall_quality * 100.0);
    
    if overall_quality > 0.85 {
        println!("  ✅ EXCELLENT - LSP quantization meets ITU standards");
    } else if overall_quality > 0.7 {
        println!("  🟡 GOOD - LSP quantization has minor distortions");
    } else if overall_quality > 0.5 {
        println!("  🟠 NEEDS IMPROVEMENT - LSP quantization has quality issues");
    } else {
        println!("  ❌ POOR - LSP quantization needs complete overhaul");
        println!("      Recommended fixes:");
        println!("      - Replace linear codebooks with ITU-T trained tables");
        println!("      - Implement proper multi-stage vector quantization");
        println!("      - Add moving average prediction");
        println!("      - Implement split-vector quantization (3-3-4)");
    }
}

/// Test pitch analysis quality and accuracy
#[test]
fn test_pitch_analysis_quality() {
    println!("🎯 Pitch Analysis Quality Evaluation");
    
    let mut pitch_analyzer = PitchAnalyzer::new();
    let mut quality_metrics = PitchQualityMetrics::new();
    
    // Generate test signals with known pitch characteristics
    let test_signals = generate_pitch_test_signals();
    
    for (signal_name, signal, expected_pitch) in &test_signals {
        println!("  Testing pitch analysis: {}", signal_name);
        
        // Analyze pitch using our implementation
        let estimated_pitch = pitch_analyzer.pitch_ol(signal, 20, 143);
        
        // Calculate quality metrics
        let pitch_error = ((estimated_pitch as f32 - *expected_pitch).abs() / *expected_pitch) * 100.0;
        let correlation_quality = evaluate_pitch_correlation(&pitch_analyzer, signal, estimated_pitch);
        let voicing_accuracy = evaluate_voicing_decision(signal, estimated_pitch);
        let stability_score = evaluate_pitch_stability(&pitch_analyzer, signal);
        
        println!("    📊 Pitch Quality Metrics:");
        println!("      Expected pitch: {:.1} samples", expected_pitch);
        println!("      Estimated pitch: {} samples", estimated_pitch);
        println!("      Pitch error: {:.1}%", pitch_error);
        println!("      Correlation quality: {:.3}", correlation_quality);
        println!("      Voicing accuracy: {:.1}%", voicing_accuracy * 100.0);
        println!("      Stability score: {:.3}", stability_score);
        
        quality_metrics.add_result(signal_name.clone(), PitchResult {
            pitch_error,
            correlation_quality,
            voicing_accuracy,
            stability_score,
        });
    }
    
    let overall_quality = quality_metrics.calculate_overall_score();
    println!("  🎯 Overall Pitch Analysis Quality: {:.1}%", overall_quality * 100.0);
    
    if overall_quality > 0.8 {
        println!("  ✅ EXCELLENT - Pitch analysis is accurate and stable");
    } else if overall_quality > 0.6 {
        println!("  🟡 GOOD - Pitch analysis has minor accuracy issues");
    } else if overall_quality > 0.4 {
        println!("  🟠 NEEDS IMPROVEMENT - Pitch analysis has significant errors");
    } else {
        println!("  ❌ POOR - Pitch analysis needs major improvements");
        println!("      Recommended fixes:");
        println!("      - Implement proper normalized correlation");
        println!("      - Add energy-based correlation scaling");
        println!("      - Implement multi-resolution search");
        println!("      - Add pitch doubling/halving detection");
        println!("      - Implement fractional pitch refinement");
    }
}

/// Test synthesis filter and output signal quality
#[test]
fn test_synthesis_quality() {
    println!("🎯 Synthesis Filter Quality Evaluation");
    
    let mut encoder = G729Encoder::new();
    let mut decoder = G729Decoder::new();
    let mut quality_metrics = SynthesisQualityMetrics::new();
    
    // Test with various input signals
    let test_signals = generate_test_signals();
    
    for (signal_name, input_signal) in &test_signals {
        println!("  Testing synthesis quality: {}", signal_name);
        
        // Process through encode/decode cycle
        let mut output_signal = Vec::new();
        let mut total_energy_preservation = 0.0;
        let mut frame_count = 0;
        
        for frame_chunk in input_signal.chunks(80) {
            if frame_chunk.len() == 80 {
                // Encode and decode
                let g729_frame = encoder.encode_frame(frame_chunk);
                let decoded_frame = decoder.decode_frame(&g729_frame);
                
                // Calculate quality metrics for this frame
                let energy_preservation = calculate_energy_preservation(frame_chunk, &decoded_frame);
                let spectral_distortion = calculate_spectral_distortion_frame(frame_chunk, &decoded_frame);
                let snr = calculate_snr(frame_chunk, &decoded_frame);
                
                total_energy_preservation += energy_preservation;
                frame_count += 1;
                
                output_signal.extend_from_slice(&decoded_frame);
            }
        }
        
        // Overall signal quality metrics
        let avg_energy_preservation = total_energy_preservation / frame_count as f32;
        let overall_snr = calculate_snr(&input_signal[..output_signal.len()], &output_signal);
        let dynamic_range = calculate_dynamic_range(&output_signal);
        let silence_preservation = evaluate_silence_preservation(&input_signal[..output_signal.len()], &output_signal);
        
        println!("    📊 Synthesis Quality Metrics:");
        println!("      Energy preservation: {:.1}%", avg_energy_preservation * 100.0);
        println!("      Overall SNR: {:.1} dB", overall_snr);
        println!("      Dynamic range: {:.1} dB", dynamic_range);
        println!("      Silence preservation: {:.1}%", silence_preservation * 100.0);
        
        quality_metrics.add_result(signal_name.clone(), SynthesisResult {
            energy_preservation: avg_energy_preservation,
            overall_snr,
            dynamic_range,
            silence_preservation,
        });
    }
    
    let overall_quality = quality_metrics.calculate_overall_score();
    println!("  🎯 Overall Synthesis Quality: {:.1}%", overall_quality * 100.0);
    
    if overall_quality > 0.8 {
        println!("  ✅ EXCELLENT - Synthesis filter produces high-quality output");
    } else if overall_quality > 0.6 {
        println!("  🟡 GOOD - Synthesis filter has minor quality issues");
    } else if overall_quality > 0.4 {
        println!("  🟠 NEEDS IMPROVEMENT - Synthesis filter has quality problems");
    } else {
        println!("  ❌ POOR - Synthesis filter produces poor quality output");
        println!("      Recommended fixes:");
        println!("      - Implement proper LSP-based formant enhancement");
        println!("      - Add spectral tilt compensation");
        println!("      - Implement long-term postfilter");
        println!("      - Fix automatic gain control algorithm");
        println!("      - Add adaptive bandwidth control");
    }
}

/// Test real ITU compliance against reference data
#[test]
fn test_real_itu_compliance() {
    println!("🎯 Real ITU-T Compliance Evaluation");
    
    // Try to load ITU reference test vectors
    let test_files = ["algthm", "fixed", "lsp", "pitch", "speech", "tame"];
    let mut compliance_metrics = ItuComplianceMetrics::new();
    
    for test_file in &test_files {
        let input_file = format!("{}.in", test_file);
        let reference_output_file = format!("{}.pst", test_file);
        
        match (parse_g729_pcm_samples(&input_file), parse_g729_pcm_samples(&reference_output_file)) {
            (Ok(input_samples), Ok(reference_output)) => {
                println!("  Testing ITU compliance: {}", test_file);
                
                // Process with our implementation
                let mut encoder = G729Encoder::new();
                let mut decoder = G729Decoder::new();
                let mut our_output = Vec::new();
                
                for frame_chunk in input_samples.chunks(80) {
                    if frame_chunk.len() == 80 {
                        let g729_frame = encoder.encode_frame(frame_chunk);
                        let decoded_frame = decoder.decode_frame(&g729_frame);
                        our_output.extend_from_slice(&decoded_frame);
                    }
                }
                
                // Compare against ITU reference
                let min_len = our_output.len().min(reference_output.len());
                let our_output_trimmed = &our_output[..min_len];
                let reference_trimmed = &reference_output[..min_len];
                
                // Calculate compliance metrics
                let similarity = calculate_signal_similarity(our_output_trimmed, reference_trimmed);
                let snr = calculate_snr(reference_trimmed, our_output_trimmed);
                let correlation = calculate_correlation(reference_trimmed, our_output_trimmed);
                let rms_error = calculate_rms_error(reference_trimmed, our_output_trimmed);
                
                println!("    📊 ITU Compliance Metrics:");
                println!("      Signal similarity: {:.1}%", similarity * 100.0);
                println!("      SNR vs reference: {:.1} dB", snr);
                println!("      Correlation: {:.3}", correlation);
                println!("      RMS error: {:.1}", rms_error);
                
                compliance_metrics.add_result(test_file.to_string(), ItuResult {
                    similarity,
                    snr,
                    correlation,
                    rms_error,
                });
            }
            _ => {
                println!("  ⚠️  ITU test files not found: {}", test_file);
            }
        }
    }
    
    if compliance_metrics.has_results() {
        let overall_compliance = compliance_metrics.calculate_overall_score();
        println!("  🎯 Real ITU-T Compliance: {:.1}%", overall_compliance * 100.0);
        
        if overall_compliance > 0.9 {
            println!("  ✅ EXCELLENT - Meets ITU-T G.729 standards");
        } else if overall_compliance > 0.8 {
            println!("  🟡 GOOD - Near ITU-T compliance with minor deviations");
        } else if overall_compliance > 0.6 {
            println!("  🟠 NEEDS IMPROVEMENT - Significant deviations from ITU-T reference");
        } else {
            println!("  ❌ POOR - Major compliance issues, substantial work needed");
        }
    } else {
        println!("  ⚠️  No ITU reference data available for compliance testing");
        println!("     Place ITU-T G.729 test vectors in the test_data directory");
    }
} 