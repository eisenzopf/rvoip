//! Audio Device Discovery Example
//!
//! This example demonstrates how to discover and inspect available audio devices
//! using the rvoip-client-core audio device abstraction layer.
//!
//! Run with: cargo run --example audio_device_discovery

use std::sync::Arc;
use rvoip_client_core::audio::{
    AudioDeviceManager, AudioDirection, AudioFormat, AudioDevice,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for better output
    tracing_subscriber::fmt::init();
    
    println!("🎵 Audio Device Discovery Example");
    println!("================================\n");
    
    // Create audio device manager
    let manager = AudioDeviceManager::new().await?;
    
    // Discover and display input devices
    println!("🎤 INPUT DEVICES (Microphones):");
    println!("------------------------------");
    
    let input_devices = manager.list_devices(AudioDirection::Input).await?;
    if input_devices.is_empty() {
        println!("  No input devices found.");
    } else {
        for (i, device_info) in input_devices.iter().enumerate() {
            println!("  {}. {} ({})", i + 1, device_info.name, device_info.id);
            println!("     Default: {}", if device_info.is_default { "Yes" } else { "No" });
            println!("     Supported Sample Rates: {:?} Hz", device_info.supported_sample_rates);
            println!("     Supported Channels: {:?}", device_info.supported_channels);
            
            // Test format compatibility
            let device = manager.create_device(&device_info.id).await?;
            test_format_compatibility(&device).await;
            println!();
        }
    }
    
    // Discover and display output devices
    println!("🔊 OUTPUT DEVICES (Speakers):");
    println!("----------------------------");
    
    let output_devices = manager.list_devices(AudioDirection::Output).await?;
    if output_devices.is_empty() {
        println!("  No output devices found.");
    } else {
        for (i, device_info) in output_devices.iter().enumerate() {
            println!("  {}. {} ({})", i + 1, device_info.name, device_info.id);
            println!("     Default: {}", if device_info.is_default { "Yes" } else { "No" });
            println!("     Supported Sample Rates: {:?} Hz", device_info.supported_sample_rates);
            println!("     Supported Channels: {:?}", device_info.supported_channels);
            
            // Test format compatibility
            let device = manager.create_device(&device_info.id).await?;
            test_format_compatibility(&device).await;
            println!();
        }
    }
    
    // Test default device access
    println!("🎯 DEFAULT DEVICE ACCESS:");
    println!("------------------------");
    
    match manager.get_default_device(AudioDirection::Input).await {
        Ok(device) => {
            println!("  ✅ Default Input: {} ({})", device.info().name, device.info().id);
        }
        Err(e) => {
            println!("  ❌ No default input device: {}", e);
        }
    }
    
    match manager.get_default_device(AudioDirection::Output).await {
        Ok(device) => {
            println!("  ✅ Default Output: {} ({})", device.info().name, device.info().id);
        }
        Err(e) => {
            println!("  ❌ No default output device: {}", e);
        }
    }
    
    println!("\n✨ Discovery complete!");
    Ok(())
}

/// Test format compatibility for a device
async fn test_format_compatibility(device: &Arc<dyn AudioDevice>) {
    let test_formats = vec![
        ("VoIP (8kHz, Mono)", AudioFormat::default_voip()),
        ("Wideband VoIP (16kHz, Mono)", AudioFormat::wideband_voip()),
        ("CD Quality (44.1kHz, Stereo)", AudioFormat::new(44100, 2, 16, 20)),
        ("Studio Quality (48kHz, Stereo)", AudioFormat::new(48000, 2, 16, 20)),
    ];
    
    print!("     Supported Formats: ");
    let mut supported_formats = Vec::new();
    
    for (name, format) in test_formats {
        if device.supports_format(&format) {
            supported_formats.push(name);
        }
    }
    
    if supported_formats.is_empty() {
        println!("None of the common formats");
    } else {
        println!("{}", supported_formats.join(", "));
    }
} 