//! # G.711 WAV File Roundtrip Test
//!
//! This module contains comprehensive roundtrip tests for the G.711 codec using real audio data.
//! The tests download actual speech samples from the internet, perform encoding and decoding
//! operations, and validate the quality of the results.
//!
//! ## What These Tests Do
//!
//! 1. **Download Real Audio**: Automatically downloads a WAV file containing real speech
//! 2. **Process Audio**: Encodes the audio using G.711 (both A-law and μ-law)  
//! 3. **Validate Quality**: Decodes the audio and measures Signal-to-Noise Ratio (SNR)
//! 4. **Save Results**: Outputs WAV files for manual quality assessment
//!
//! ## Test Audio File
//!
//! - **Source**: VoIP Troubleshooter reference audio
//! - **Format**: 16-bit PCM, 8 kHz, mono
//! - **Content**: American English speech sample
//! - **Duration**: ~33.6 seconds (268,985 samples)
//! - **URL**: https://www.voiptroubleshooter.com/open_speech/american/OSR_us_000_0010_8k.wav
//!
//! ## Quality Metrics
//!
//! The tests measure and validate:
//! - **SNR (Signal-to-Noise Ratio)**: Measures compression quality
//! - **Sample Preservation**: Ensures no sample corruption
//! - **Dynamic Range**: Validates proper handling of audio levels
//!
//! ## Expected Results
//!
//! For production-quality G.711 implementation:
//! - **A-law SNR**: >37 dB (excellent quality)
//! - **μ-law SNR**: >37 dB (excellent quality) 
//! - **Sample Integrity**: 99-100% non-zero samples preserved
//! - **Compression Ratio**: 1:1 (G.711 characteristic)
//!
//! ## Generated Files
//!
//! The tests create several files in `test_data/`:
//! - `OSR_us_000_0010_8k.wav` - Original downloaded audio
//! - `OSR_us_000_0010_8k_roundtrip_alaw.wav` - A-law processed result
//! - `OSR_us_000_0010_8k_roundtrip_ulaw.wav` - μ-law processed result
//!
//! ## Running the Tests
//!
//! ```bash
//! # Run both A-law and μ-law roundtrip tests
//! cargo test wav_roundtrip_test -- --nocapture
//!
//! # Run only A-law test
//! cargo test test_g711_alaw_roundtrip_real_audio -- --nocapture
//!
//! # Run only μ-law test  
//! cargo test test_g711_ulaw_roundtrip_real_audio -- --nocapture
//! ```
//!
//! The `--nocapture` flag shows detailed output including SNR measurements and file paths.

use crate::codecs::g711::*;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

/// WAV file header structure
#[repr(C, packed)]
struct WavHeader {
    // RIFF chunk
    riff_id: [u8; 4],      // "RIFF"
    file_size: u32,        // File size - 8
    wave_id: [u8; 4],      // "WAVE"
    
    // fmt chunk
    fmt_id: [u8; 4],       // "fmt "
    fmt_size: u32,         // 16 for PCM
    audio_format: u16,     // 1 for PCM
    num_channels: u16,     // 1 for mono
    sample_rate: u32,      // 8000 Hz
    byte_rate: u32,        // sample_rate * num_channels * bits_per_sample / 8
    block_align: u16,      // num_channels * bits_per_sample / 8
    bits_per_sample: u16,  // 16 bits
    
    // data chunk
    data_id: [u8; 4],      // "data"
    data_size: u32,        // Number of bytes in data
}

impl WavHeader {
    fn new(num_samples: usize) -> Self {
        let data_size = (num_samples * 2) as u32; // 16-bit samples
        let file_size = data_size + 36; // 44 - 8
        
        WavHeader {
            riff_id: *b"RIFF",
            file_size,
            wave_id: *b"WAVE",
            fmt_id: *b"fmt ",
            fmt_size: 16,
            audio_format: 1,
            num_channels: 1,
            sample_rate: 8000,
            byte_rate: 16000, // 8000 * 1 * 16 / 8
            block_align: 2,   // 1 * 16 / 8
            bits_per_sample: 16,
            data_id: *b"data",
            data_size,
        }
    }
}

/// Download a file from a URL
async fn download_file(url: &str, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Simple HTTP client without external dependencies
    let response = std::process::Command::new("curl")
        .arg("-s")  // Silent
        .arg("-L")  // Follow redirects
        .arg("-o")
        .arg(path)
        .arg(url)
        .output()?;
    
    if !response.status.success() {
        return Err(format!("Failed to download file: {}", 
            String::from_utf8_lossy(&response.stderr)).into());
    }
    
    Ok(())
}

/// Read a WAV file and return the samples
fn read_wav_file(path: &Path) -> Result<Vec<i16>, Box<dyn std::error::Error>> {
    let mut file = File::open(path)?;
    
    // Read and validate WAV header
    let mut header_bytes = [0u8; 44];
    file.read_exact(&mut header_bytes)?;
    
    // Basic validation
    if &header_bytes[0..4] != b"RIFF" || &header_bytes[8..12] != b"WAVE" {
        return Err("Not a valid WAV file".into());
    }
    
    // Check format
    let audio_format = u16::from_le_bytes([header_bytes[20], header_bytes[21]]);
    let num_channels = u16::from_le_bytes([header_bytes[22], header_bytes[23]]);
    let sample_rate = u32::from_le_bytes([header_bytes[24], header_bytes[25], header_bytes[26], header_bytes[27]]);
    let bits_per_sample = u16::from_le_bytes([header_bytes[34], header_bytes[35]]);
    
    if audio_format != 1 {
        return Err("Only PCM format is supported".into());
    }
    if num_channels != 1 {
        return Err("Only mono audio is supported".into());
    }
    if sample_rate != 8000 {
        return Err("Only 8000 Hz sample rate is supported".into());
    }
    if bits_per_sample != 16 {
        return Err("Only 16-bit samples are supported".into());
    }
    
    // Read samples
    let data_size = u32::from_le_bytes([header_bytes[40], header_bytes[41], header_bytes[42], header_bytes[43]]);
    let num_samples = (data_size / 2) as usize;
    let mut samples = vec![0i16; num_samples];
    
    for i in 0..num_samples {
        let mut bytes = [0u8; 2];
        file.read_exact(&mut bytes)?;
        samples[i] = i16::from_le_bytes(bytes);
    }
    
    Ok(samples)
}

/// Write samples to a WAV file
fn write_wav_file(path: &Path, samples: &[i16]) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::create(path)?;
    
    // Create header
    let header = WavHeader::new(samples.len());
    
    // Write header (manually to avoid alignment issues)
    file.write_all(&header.riff_id)?;
    file.write_all(&header.file_size.to_le_bytes())?;
    file.write_all(&header.wave_id)?;
    file.write_all(&header.fmt_id)?;
    file.write_all(&header.fmt_size.to_le_bytes())?;
    file.write_all(&header.audio_format.to_le_bytes())?;
    file.write_all(&header.num_channels.to_le_bytes())?;
    file.write_all(&header.sample_rate.to_le_bytes())?;
    file.write_all(&header.byte_rate.to_le_bytes())?;
    file.write_all(&header.block_align.to_le_bytes())?;
    file.write_all(&header.bits_per_sample.to_le_bytes())?;
    file.write_all(&header.data_id)?;
    file.write_all(&header.data_size.to_le_bytes())?;
    
    // Write samples
    for sample in samples {
        file.write_all(&sample.to_le_bytes())?;
    }
    
    Ok(())
}

/// Calculate signal-to-noise ratio between two signals
fn calculate_snr(original: &[i16], decoded: &[i16]) -> f64 {
    assert_eq!(original.len(), decoded.len(), "Signals must have same length");
    
    let mut signal_power = 0i64;
    let mut noise_power = 0i64;
    
    for i in 0..original.len() {
        let signal = original[i] as i64;
        let noise = (original[i] as i64) - (decoded[i] as i64);
        
        signal_power += signal * signal;
        noise_power += noise * noise;
    }
    
    if noise_power == 0 {
        return f64::INFINITY;
    }
    
    10.0 * ((signal_power as f64) / (noise_power as f64)).log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio;

    #[tokio::test]
    async fn test_g711_alaw_roundtrip_real_audio() {
        test_g711_roundtrip_real_audio(G711Variant::ALaw, "alaw").await;
    }

    #[tokio::test]
    async fn test_g711_ulaw_roundtrip_real_audio() {
        test_g711_roundtrip_real_audio(G711Variant::MuLaw, "ulaw").await;
    }

    async fn test_g711_roundtrip_real_audio(variant: G711Variant, variant_name: &str) {
        const WAV_URL: &str = "https://www.voiptroubleshooter.com/open_speech/american/OSR_us_000_0010_8k.wav";
        
        // Create test data directory if it doesn't exist
        let test_dir = Path::new("src/codecs/g711/tests/test_data");
        std::fs::create_dir_all(test_dir).expect("Failed to create test directory");
        
        // Download the original WAV file
        let original_wav_path = test_dir.join("OSR_us_000_0010_8k.wav");
        
        // Only download if file doesn't exist (to avoid unnecessary downloads)
        if !original_wav_path.exists() {
            println!("Downloading WAV file from: {}", WAV_URL);
            download_file(WAV_URL, &original_wav_path).await
                .expect("Failed to download WAV file");
            println!("Downloaded WAV file to: {:?}", original_wav_path);
        } else {
            println!("Using existing WAV file: {:?}", original_wav_path);
        }
        
        // Read the original WAV file
        let original_samples = read_wav_file(&original_wav_path)
            .expect("Failed to read WAV file");
        
        println!("Loaded {} samples from WAV file", original_samples.len());
        
        // Create G.711 codec
        let codec = G711Codec::new(variant);
        
        // Encode the samples
        let encoded = codec.compress(&original_samples)
            .expect("Failed to encode samples");
        
        println!("Encoded {} samples to {} bytes using {}", 
            original_samples.len(), encoded.len(), variant_name);
        
        // Decode the samples back
        let decoded_samples = codec.expand(&encoded)
            .expect("Failed to decode samples");
        
        println!("Decoded {} bytes to {} samples using {}", 
            encoded.len(), decoded_samples.len(), variant_name);
        
        // Verify we have the same number of samples
        assert_eq!(original_samples.len(), decoded_samples.len(), 
            "Sample count mismatch after roundtrip");
        
        // Calculate SNR
        let snr = calculate_snr(&original_samples, &decoded_samples);
        println!("Signal-to-Noise Ratio: {:.2} dB", snr);
        
        // G.711 should have reasonable SNR (typically 30-40 dB for speech)
        assert!(snr > 20.0, "SNR too low: {:.2} dB", snr);
        
        // Save the roundtrip result
        let output_wav_path = test_dir.join(format!("OSR_us_000_0010_8k_roundtrip_{}.wav", variant_name));
        write_wav_file(&output_wav_path, &decoded_samples)
            .expect("Failed to write output WAV file");
        
        println!("Saved roundtrip result to: {:?}", output_wav_path);
        
        // Basic sanity checks
        assert!(!decoded_samples.is_empty(), "Decoded samples should not be empty");
        
        // Check that not all samples are zero (silence)
        let non_zero_samples = decoded_samples.iter().filter(|&&s| s != 0).count();
        assert!(non_zero_samples > original_samples.len() / 10, 
            "Too many zero samples, audio might be corrupted");
        
        println!("✓ G.711 {} roundtrip test passed!", variant_name);
        println!("  - Original samples: {}", original_samples.len());
        println!("  - Encoded bytes: {}", encoded.len());
        println!("  - Compression ratio: 1:1 (G.711 is 1:1)");
        println!("  - SNR: {:.2} dB", snr);
        println!("  - Non-zero samples: {}%", (non_zero_samples * 100) / original_samples.len());
    }
} 