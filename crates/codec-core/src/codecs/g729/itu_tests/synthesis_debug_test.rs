//! Debug test for synthesis filter energy issues

use super::super::src::encoder::G729Encoder;
use super::super::src::decoder::G729Decoder;
use super::super::src::types::*;

#[test]
fn debug_synthesis_filter_energy() {
    println!("🔍 Debug: Synthesis Filter Energy Issues");
    
    let mut encoder = G729Encoder::new();
    let mut decoder = G729Decoder::new();
    
    // Simple test signal
    let test_signal: Vec<i16> = (0..80).map(|i| {
        (1000.0 * (2.0 * std::f32::consts::PI * i as f32 / 40.0).sin()) as i16
    }).collect();
    
    println!("Input signal energy: {:.1}", calculate_energy(&test_signal));
    println!("Input signal first 10 samples: {:?}", &test_signal[..10]);
    
    // Encode
    let g729_frame = encoder.encode_frame(&test_signal);
    println!("G729 frame encoded successfully");
    
    // Manual decoding with detailed logging
    println!("\n🔧 Manual Decoding Debug:");
    
    // Test gain dequantization
    for (i, subframe) in g729_frame.subframes.iter().enumerate() {
        println!("Subframe {}: pitch_lag={}, positions={:?}, gain_index={}", 
                i, subframe.pitch_lag, subframe.positions, subframe.gain_index);
        
        // Test fixed codebook innovation building
        let mut fixed_exc = [0i16; 40];
        let acelp_analyzer = super::super::src::acelp::AcelpAnalyzer::new();
        acelp_analyzer.build_innovation(
            &subframe.positions,
            &subframe.signs,
            subframe.gain_index,
            &mut fixed_exc,
        );
        
        let fixed_energy = calculate_energy(&fixed_exc);
        println!("  Fixed excitation energy: {:.1}", fixed_energy);
        println!("  Fixed excitation first 10: {:?}", &fixed_exc[..10]);
        
        if fixed_energy < 1.0 {
            println!("  ⚠️  Fixed excitation energy is too low!");
        }
    }
    
    // Decode normally
    let decoded_signal = decoder.decode_frame(&g729_frame);
    
    println!("\nOutput signal energy: {:.1}", calculate_energy(&decoded_signal));
    println!("Output signal first 10 samples: {:?}", &decoded_signal[..10]);
    
    // More lenient test - just check for any non-zero output
    let has_non_zero = decoded_signal.iter().any(|&x| x.abs() > 1);
    println!("Has non-zero output: {}", has_non_zero);
    
    if !has_non_zero {
        println!("❌ Decoder producing silence - investigating...");
        
        // Check if the problem is in gain reconstruction
        println!("Investigating gain issues...");
    }
}

#[test]
fn debug_gain_quantization_issue() {
    println!("🔍 Debug: Gain Quantization Issue");
    
    let mut encoder = G729Encoder::new();
    
    // High energy test signal to force higher gains
    let high_energy_signal = vec![8000i16; 80]; // Very high amplitude DC signal
    
    println!("High energy signal energy: {:.1}", calculate_energy(&high_energy_signal));
    
    let g729_frame = encoder.encode_frame(&high_energy_signal);
    
    for (i, subframe) in g729_frame.subframes.iter().enumerate() {
        println!("High Energy Subframe {}: gain_index={}", i, subframe.gain_index);
        
        // Test what gain this index maps to
        let mut acelp_analyzer = super::super::src::acelp::AcelpAnalyzer::new();
        let mut innovation = [0i16; 40];
        acelp_analyzer.build_innovation(
            &subframe.positions,
            &subframe.signs,
            subframe.gain_index,
            &mut innovation,
        );
        
        println!("  Resulting innovation energy: {:.1}", calculate_energy(&innovation));
        println!("  Innovation first few: {:?}", &innovation[..4]);
        
        // Check what the optimal gain should be vs what index gives
        println!("  Gain index {} maps to gain range:", subframe.gain_index);
        for test_index in 0..10 {
            let mut test_innovation = [0i16; 40];
            acelp_analyzer.build_innovation(
                &subframe.positions,
                &subframe.signs,
                test_index,
                &mut test_innovation,
            );
            let test_energy = calculate_energy(&test_innovation);
            println!("    Index {}: energy {:.1}", test_index, test_energy);
        }
    }
}

#[test]
fn debug_excitation_components() {
    println!("🔍 Debug: Individual Excitation Components");
    
    let mut encoder = G729Encoder::new();
    
    // Simple DC signal to test gain
    let dc_signal = vec![1000i16; 80];
    
    println!("DC signal energy: {:.1}", calculate_energy(&dc_signal));
    
    let g729_frame = encoder.encode_frame(&dc_signal);
    
    for (i, subframe) in g729_frame.subframes.iter().enumerate() {
        println!("Subframe {}: gain_index={}, positions={:?}", 
                i, subframe.gain_index, subframe.positions);
        
        // Test innovation building with simple positions
        let mut innovation = [0i16; 40];
        let acelp_analyzer = super::super::src::acelp::AcelpAnalyzer::new();
        acelp_analyzer.build_innovation(
            &subframe.positions,
            &subframe.signs,
            subframe.gain_index,
            &mut innovation,
        );
        
        println!("  Innovation energy: {:.1}", calculate_energy(&innovation));
        println!("  Non-zero samples: {}", innovation.iter().filter(|&&x| x != 0).count());
        
        // Check individual pulse values
        for j in 0..4 {
            let pos = subframe.positions[j];
            if pos < innovation.len() {
                println!("    Pulse {}: pos={}, value={}", j, pos, innovation[pos]);
            }
        }
    }
}

fn calculate_energy(signal: &[i16]) -> f32 {
    signal.iter().map(|&x| (x as f32).powi(2)).sum::<f32>() / signal.len() as f32
} 