//! Debug test for gain reconstruction issues

use super::super::src::energy_preservation::reconstruct_gains_itu_compliant;
use super::super::src::encoder::G729Encoder;

#[test]
fn debug_gain_reconstruction_issue() {
    println!("🔍 Debug: Gain Reconstruction Issue");
    
    // Test what gains we get from different indices
    println!("Gain Index -> (Adaptive Q14, Fixed Q1) mapping:");
    for index in 0..20 {
        let (adaptive, fixed) = reconstruct_gains_itu_compliant(index, 10000);
        println!("  Index {:2}: adaptive={:5} (Q14: {:.3}), fixed={:4} (Q1: {:.1})", 
                index, adaptive, adaptive as f32 / 16384.0, fixed, fixed as f32 / 2.0);
    }
    
    // Test what happens when we encode a high-energy signal
    let mut encoder = G729Encoder::new();
    let high_energy_signal: Vec<i16> = (0..80).map(|i| {
        (5000.0 * (2.0 * std::f32::consts::PI * i as f32 / 16.0).sin()) as i16
    }).collect();
    
    let input_energy = calculate_energy(&high_energy_signal);
    println!("\nHigh energy input signal: energy={:.1}", input_energy);
    
    let frame = encoder.encode_frame(&high_energy_signal);
    
    for (i, subframe) in frame.subframes.iter().enumerate() {
        println!("Subframe {}: gain_index={}", i, subframe.gain_index);
        let (adaptive, fixed) = reconstruct_gains_itu_compliant(subframe.gain_index, 10000);
        println!("  -> adaptive={} (Q14: {:.3}), fixed={} (Q1: {:.1})", 
                adaptive, adaptive as f32 / 16384.0, fixed, fixed as f32 / 2.0);
    }
    
    // Test what minimum gains should be for reasonable output
    println!("\nAnalyzing required gains for energy preservation:");
    let target_output_energy = input_energy * 0.5; // Target: preserve 50% of energy
    let excitation_energy = 50000.0; // Typical excitation energy from our tests
    let required_synthesis_gain = (target_output_energy / excitation_energy).sqrt();
    println!("Required synthesis gain: {:.3}", required_synthesis_gain);
    println!("Required gain in Q14: {:.0}", required_synthesis_gain * 16384.0);
    println!("Required gain in Q1: {:.0}", required_synthesis_gain * 2.0);
}

fn calculate_energy(signal: &[i16]) -> f32 {
    signal.iter().map(|&x| (x as f32).powi(2)).sum::<f32>() / signal.len() as f32
} 