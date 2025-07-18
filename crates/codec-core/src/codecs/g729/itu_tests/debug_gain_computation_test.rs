//! Debug test for gain computation details

use super::super::src::acelp::AcelpAnalyzer;
use super::super::src::encoder::G729Encoder;

#[test]
fn debug_gain_computation_details() {
    println!("🔍 Debug: Gain Computation Step-by-Step");
    
    let mut encoder = G729Encoder::new();
    let high_energy_signal: Vec<i16> = (0..80).map(|i| {
        (5000.0 * (2.0 * std::f32::consts::PI * i as f32 / 16.0).sin()) as i16
    }).collect();
    
    println!("Input signal energy: {:.1}", calculate_energy(&high_energy_signal));
    
    // Encode to get the frame
    let frame = encoder.encode_frame(&high_energy_signal);
    
    // Now manually test the ACELP gain computation
    let mut acelp = AcelpAnalyzer::new();
    
    // Create a realistic target signal (what the encoder would see)
    let target_signal: Vec<i16> = (0..40).map(|i| {
        (3000.0 * (2.0 * std::f32::consts::PI * i as f32 / 8.0).sin()) as i16
    }).collect();
    
    let target_energy = calculate_energy(&target_signal);
    println!("Target signal energy: {:.1}", target_energy);
    
    // Create a realistic filtered code signal  
    let filtered_code: Vec<i16> = (0..40).map(|i| {
        (1500.0 * (2.0 * std::f32::consts::PI * i as f32 / 12.0).sin()) as i16
    }).collect();
    
    println!("Filtered code energy: {:.1}", calculate_energy(&filtered_code));
    
    // Manually compute the gain index using the ACELP logic
    let gain_index = test_compute_gain_index(&target_signal, &filtered_code);
    
    println!("Computed gain index: {}", gain_index);
    
    // Test what our actual encoder produced
    for (i, subframe) in frame.subframes.iter().enumerate() {
        println!("Encoder subframe {}: gain_index={}", i, subframe.gain_index);
    }
}

fn test_compute_gain_index(target: &[i16], filtered_code: &[i16]) -> usize {
    println!("\n--- Manual Gain Computation ---");
    
    // Compute correlation between target and filtered code
    let mut num = 0i32;
    let mut den = 0i32;
    
    for i in 0..40 {
        let target_val = target[i] as i32;
        let code_val = filtered_code[i] as i32;
        
        num += target_val * code_val;
        den += code_val * code_val;
    }
    
    println!("Correlation num: {}", num);
    println!("Energy den: {}", den);
    
    // Compute target energy for reference
    let mut target_energy = 0i32;
    for &sample in target {
        target_energy += (sample as i32) * (sample as i32);
    }
    
    println!("Target energy: {}", target_energy);
    
    // Compute optimal gain
    let optimal_gain = if den > 0 {
        let raw_gain = (num / den.max(1)).max(0);
        println!("Raw gain: {}", raw_gain);
        
        let energy_scale = if target_energy > 1000000 { 
            50  // Very high energy signals need massive scaling
        } else if target_energy > 100000 { 
            25  // High energy signals need major scaling
        } else if target_energy > 10000 { 
            12  // Medium energy needs significant scaling  
        } else { 
            6   // Low energy needs moderate scaling
        };
        
        println!("Energy scale factor: {}", energy_scale);
        
        let scaled_gain = (raw_gain * energy_scale as i32).min(20000) as i16;
        let final_gain = scaled_gain.max(8000);
        
        println!("Scaled gain: {}", scaled_gain);
        println!("Final optimal gain: {}", final_gain);
        
        final_gain
    } else {
        println!("Zero denominator, using default gain: 8000");
        8000
    };
    
    // Find best index
    let best_index = find_best_gain_index_debug(optimal_gain);
    
    // Energy-based index
    let energy_based_index = if target_energy > 1000000 {
        (32 + (target_energy / 200000).min(31)) as usize
    } else if target_energy > 100000 {
        (16 + (target_energy / 50000).min(15)) as usize
    } else if target_energy > 10000 {
        (8 + (target_energy / 5000).min(7)) as usize
    } else {
        (4 + (target_energy / 2000).min(3)) as usize
    };
    
    println!("Best match index: {}", best_index);
    println!("Energy-based index: {}", energy_based_index);
    
    // Final selection logic
    let final_index = if target_energy > 50000 && energy_based_index > best_index {
        energy_based_index.min(80)
    } else if best_index == 0 {
        energy_based_index.max(4).min(80)
    } else {
        best_index
    };
    
    println!("Final selected index: {}", final_index);
    
    final_index
}

fn find_best_gain_index_debug(optimal_gain: i16) -> usize {
    println!("Finding best index for optimal_gain: {}", optimal_gain);
    
    let mut best_index = 0;
    let mut min_error = i32::MAX;
    
    for index in 0..20 {  // Check first 20 indices
        let codebook_gain = match index {
            0..=15 => (8000 + index * 800) as i16,
            16..=31 => (12000 + (index - 16) * 400) as i16,
            _ => 16000,
        };
        
        let error = (optimal_gain as i32 - codebook_gain as i32).abs();
        
        if index < 10 {  // Show first 10 for debugging
            println!("  Index {}: codebook_gain={}, error={}", index, codebook_gain, error);
        }
        
        if error < min_error {
            min_error = error;
            best_index = index;
        }
    }
    
    best_index
}

fn calculate_energy(signal: &[i16]) -> f32 {
    signal.iter().map(|&x| (x as f32).powi(2)).sum::<f32>() / signal.len() as f32
} 