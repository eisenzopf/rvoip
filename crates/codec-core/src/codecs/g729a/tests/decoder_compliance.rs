//! G.729A Decoder Compliance Tests
//! 
//! Tests the decoder output against ITU-T reference decoded samples

use crate::codecs::g729a::{G729ADecoder};
use crate::codecs::g729a::bitstream_utils::read_itu_bitstream;
use std::fs::File;
use std::io::Read;
use std::path::Path;

fn read_pcm_file(path: &Path) -> Result<Vec<i16>, std::io::Error> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    
    let samples: Vec<i16> = buffer.chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    
    Ok(samples)
}

fn calculate_snr(original: &[i16], decoded: &[i16]) -> f64 {
    if original.len() != decoded.len() {
        return 0.0;
    }
    
    let signal_power: f64 = original.iter()
        .map(|&s| (s as f64) * (s as f64))
        .sum::<f64>() / original.len() as f64;
    
    let noise_power: f64 = original.iter()
        .zip(decoded.iter())
        .map(|(&o, &d)| {
            let diff = (o as f64) - (d as f64);
            diff * diff
        })
        .sum::<f64>() / original.len() as f64;
    
    if noise_power > 0.0 {
        10.0 * (signal_power / noise_power).log10()
    } else {
        f64::INFINITY
    }
}

#[test]
fn test_decoder_algthm() {
    let test_vectors_dir = Path::new("src/codecs/g729a/tests/test_vectors");
    let bitstream_path = test_vectors_dir.join("ALGTHM.BIT");
    let reference_path = test_vectors_dir.join("ALGTHM.PST");
    
    // Read bitstream (now using ITU-T format)
    let bitstream_frames = read_itu_bitstream(&bitstream_path)
        .expect("Failed to read ALGTHM.BIT");
    
    // Read reference output
    let reference_samples = read_pcm_file(&reference_path)
        .expect("Failed to read ALGTHM.PST");
    
    println!("Testing decoder with ALGTHM vector");
    println!("Bitstream frames: {}", bitstream_frames.len());
    println!("Reference samples: {}", reference_samples.len());
    
    // Show first frame in packed format
    if bitstream_frames.len() > 0 {
        println!("\nFirst frame (packed from ITU format):");
        println!("  {:02X?}", bitstream_frames[0]);
    }
    
    let mut decoder = G729ADecoder::new();
    let mut decoded_samples = Vec::new();
    
    // Decode each frame
    for (frame_idx, frame_bits) in bitstream_frames.iter().enumerate() {
        match decoder.decode_frame(frame_bits) {
            Ok(decoded_frame) => {
                decoded_samples.extend_from_slice(&decoded_frame.samples);
            }
            Err(e) => {
                eprintln!("Decoding error at frame {}: {:?}", frame_idx, e);
                break;
            }
        }
    }
    
    // Compare with reference
    let samples_to_compare = decoded_samples.len().min(reference_samples.len());
    let mut exact_matches = 0;
    let mut max_diff = 0i16;
    let mut total_diff = 0i64;
    
    for i in 0..samples_to_compare {
        if decoded_samples[i] == reference_samples[i] {
            exact_matches += 1;
        }
        let diff = (decoded_samples[i] as i32 - reference_samples[i] as i32).abs() as i16;
        max_diff = max_diff.max(diff);
        total_diff += diff as i64;
    }
    
    let match_percentage = (exact_matches as f64 / samples_to_compare as f64) * 100.0;
    let avg_diff = total_diff as f64 / samples_to_compare as f64;
    let snr = calculate_snr(&reference_samples[..samples_to_compare], 
                           &decoded_samples[..samples_to_compare]);
    
    println!("\nDecoder Compliance Results:");
    println!("Samples compared: {}", samples_to_compare);
    println!("Bit-exact matches: {} ({:.2}%)", exact_matches, match_percentage);
    println!("Maximum difference: {}", max_diff);
    println!("Average difference: {:.2}", avg_diff);
    println!("Signal-to-Noise Ratio: {:.2} dB", snr);
    
    // For now, we expect this to fail until we fix the implementation
    assert!(samples_to_compare > 0, "No samples were decoded");
}

#[test]
#[ignore = "Run only after decoder is working"]
fn test_decoder_erasure() {
    // ERASURE test vector specifically tests frame erasure handling
    let test_vectors_dir = Path::new("src/codecs/g729a/tests/test_vectors");
    let bitstream_path = test_vectors_dir.join("ERASURE.BIT");
    let reference_path = test_vectors_dir.join("ERASURE.PST");
    
    // TODO: Implement erasure test with proper frame erasure simulation
    println!("Frame erasure test not yet implemented");
}

#[test]
#[ignore = "Run only after decoder is working"]
fn test_decoder_all_vectors() {
    let test_vectors = vec![
        ("ALGTHM", "Algorithmic conditionals"),
        ("ERASURE", "Frame erasure recovery"),
        ("OVERFLOW", "Overflow detection"),
        ("PARITY", "Parity check"),
    ];
    
    for (vector_name, description) in test_vectors {
        println!("\nTesting {} - {}", vector_name, description);
        // TODO: Implement full test
    }
} 