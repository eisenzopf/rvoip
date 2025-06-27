//! Debug AGC v2 Filter Creation
//! 
//! Test to isolate which biquad filter is causing the "OutsideNyquist" error

use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type};

#[tokio::test]
async fn debug_agc_filter_creation() {
    println!("🔍 Debugging AGC v2 Filter Creation");
    println!("====================================");
    
    let sample_rate = 16000.0_f32;
    let nyquist = sample_rate / 2.0;
    println!("Sample rate: {} Hz, Nyquist frequency: {} Hz", sample_rate, nyquist);
    
    let crossover_frequencies = vec![300.0, 3000.0];
    println!("Crossover frequencies: {:?}", crossover_frequencies);
    
    // Test filter creation for each band
    let num_bands = crossover_frequencies.len() + 1;
    println!("Number of bands: {}", num_bands);
    
    for band_idx in 0..num_bands {
        println!("\n--- Band {} ---", band_idx);
        
        if band_idx == 0 {
            // First band: low-pass at first crossover frequency
            let freq = crossover_frequencies[0];
            println!("Creating low-pass filter at {} Hz", freq);
            
            match Coefficients::<f32>::from_params(
                Type::LowPass,
                freq.hz(),
                sample_rate.hz(),
                biquad::Q_BUTTERWORTH_F32,
            ) {
                Ok(coeffs) => {
                    println!("✅ Low-pass filter created successfully");
                    let _filter = DirectForm1::<f32>::new(coeffs);
                },
                Err(e) => {
                    println!("❌ Low-pass filter failed: {:?}", e);
                }
            }
            
        } else if band_idx == num_bands - 1 {
            // Last band: high-pass at last crossover frequency
            let freq = crossover_frequencies[crossover_frequencies.len() - 1];
            println!("Creating high-pass filter at {} Hz", freq);
            
            match Coefficients::<f32>::from_params(
                Type::HighPass,
                freq.hz(),
                sample_rate.hz(),
                biquad::Q_BUTTERWORTH_F32,
            ) {
                Ok(coeffs) => {
                    println!("✅ High-pass filter created successfully");
                    let _filter = DirectForm1::<f32>::new(coeffs);
                },
                Err(e) => {
                    println!("❌ High-pass filter failed: {:?}", e);
                }
            }
            
        } else {
            // Middle bands: band-pass between two crossover frequencies
            let low_freq = crossover_frequencies[band_idx - 1];
            let high_freq = crossover_frequencies[band_idx];
            println!("Creating band-pass filter: {} Hz to {} Hz", low_freq, high_freq);
            
            // High-pass at lower frequency
            println!("  - High-pass at {} Hz", low_freq);
            match Coefficients::<f32>::from_params(
                Type::HighPass,
                low_freq.hz(),
                sample_rate.hz(),
                biquad::Q_BUTTERWORTH_F32,
            ) {
                Ok(coeffs) => {
                    println!("    ✅ High-pass filter created successfully");
                    let _filter = DirectForm1::<f32>::new(coeffs);
                },
                Err(e) => {
                    println!("    ❌ High-pass filter failed: {:?}", e);
                }
            }
            
            // Low-pass at higher frequency
            println!("  - Low-pass at {} Hz", high_freq);
            match Coefficients::<f32>::from_params(
                Type::LowPass,
                high_freq.hz(),
                sample_rate.hz(),
                biquad::Q_BUTTERWORTH_F32,
            ) {
                Ok(coeffs) => {
                    println!("    ✅ Low-pass filter created successfully");
                    let _filter = DirectForm1::<f32>::new(coeffs);
                },
                Err(e) => {
                    println!("    ❌ Low-pass filter failed: {:?}", e);
                }
            }
        }
    }
    
    // Test different Q factor values
    println!("\n🧪 Testing Q factor values...");
    let test_freq = 1000.0;
    
    let q_values = vec![
        ("BUTTERWORTH", biquad::Q_BUTTERWORTH_F32),
        ("SQRT_2", std::f32::consts::SQRT_2),
        ("1.0", 1.0),
        ("0.707", 0.707),
        ("0.5", 0.5),
    ];
    
    for (name, q_value) in q_values {
        println!("Testing Q = {} ({})", q_value, name);
        match Coefficients::<f32>::from_params(
            Type::LowPass,
            test_freq.hz(),
            sample_rate.hz(),
            q_value,
        ) {
            Ok(_) => println!("  ✅ Q = {} works", q_value),
            Err(e) => println!("  ❌ Q = {} failed: {:?}", q_value, e),
        }
    }
    
    // Test different sample rates
    println!("\n--- Testing different sample rates ---");
    let test_sample_rates = vec![8000.0, 16000.0, 32000.0, 48000.0];
    let fixed_freq = 1000.0;
    
    for sr in test_sample_rates {
        println!("Sample rate: {} Hz, Test freq: {} Hz, Ratio: {}", 
                sr, fixed_freq, fixed_freq / sr);
        match Coefficients::<f32>::from_params(
            Type::LowPass,
            fixed_freq.hz(),
            sr.hz(),
            biquad::Q_BUTTERWORTH_F32,
        ) {
            Ok(_) => println!("  ✅ Works with {} Hz sample rate", sr),
            Err(e) => println!("  ❌ Failed with {} Hz sample rate: {:?}", sr, e),
        }
    }
    
    println!("\n✅ Filter debug test completed");
} 