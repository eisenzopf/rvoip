//! Test Export Demo
//!
//! This example verifies that VAD and AEC components are properly exported
//! and accessible from the public API.

use rvoip_media_core::prelude::*;
use rvoip_media_core::processing::{
    VadConfig, AecConfig, AgcConfig,
    VoiceActivityDetector, AcousticEchoCanceller, AutomaticGainControl
};

fn main() -> Result<()> {
    println!("🧪 Testing VAD and AEC exports...");
    
    // Test that we can create VAD configuration
    let vad_config = VadConfig::default();
    println!("✓ VadConfig accessible with defaults:");
    println!("  - energy_threshold: {:.3}", vad_config.energy_threshold);
    println!("  - zcr_threshold: {:.3}", vad_config.zcr_threshold);
    
    // Test that we can create AEC configuration
    let aec_config = AecConfig::default();
    println!("✓ AecConfig accessible with defaults:");
    println!("  - filter_length: {}", aec_config.filter_length);
    println!("  - step_size: {:.3}", aec_config.step_size);
    
    // Test that we can create AGC configuration
    let agc_config = AgcConfig::default();
    println!("✓ AgcConfig accessible with defaults:");
    println!("  - target_level: {:.2}", agc_config.target_level);
    println!("  - compression_ratio: {:.1}", agc_config.compression_ratio);
    
    // Test that we can instantiate components
    match VoiceActivityDetector::new(vad_config) {
        Ok(_) => println!("✓ VoiceActivityDetector can be instantiated"),
        Err(e) => {
            println!("✗ VoiceActivityDetector error: {}", e);
            return Err(e);
        }
    }
    
    match AcousticEchoCanceller::new(aec_config) {
        Ok(_) => println!("✓ AcousticEchoCanceller can be instantiated"),
        Err(e) => {
            println!("✗ AcousticEchoCanceller error: {}", e);
            return Err(e);
        }
    }
    
    match AutomaticGainControl::new(agc_config) {
        Ok(_) => println!("✓ AutomaticGainControl can be instantiated"),
        Err(e) => {
            println!("✗ AutomaticGainControl error: {}", e);
            return Err(e);
        }
    }
    
    // Test a simple audio frame creation
    let test_frame = AudioFrame::new(
        vec![100i16; 160], // 160 samples of audio
        8000,              // 8kHz sample rate
        1,                 // mono
        0                  // timestamp
    );
    
    println!("✓ AudioFrame can be created: {} samples at {}Hz", 
             test_frame.samples.len(), test_frame.sample_rate);
    
    println!("✨ All audio processing components are properly exported and functional!");
    println!("");
    println!("🎯 Summary:");
    println!("  • Voice Activity Detection (VAD) - ✓ Available");
    println!("  • Acoustic Echo Cancellation (AEC) - ✓ Available");
    println!("  • Automatic Gain Control (AGC) - ✓ Available");
    println!("  • Configuration types - ✓ Exported");
    println!("  • Component instantiation - ✓ Working");
    
    Ok(())
} 