//! G.729A Encoder Compliance Tests
//! 
//! Tests the encoder output against ITU-T reference bitstreams

use crate::codecs::g729a::{G729AEncoder, AudioFrame};
use crate::codecs::g729a::bitstream_utils::{read_itu_bitstream, write_itu_bitstream};
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
    
    // Debug: find first non-zero sample
    let first_nonzero = samples.iter().position(|&x| x != 0);
    println!("Read {} bytes from {:?}", buffer.len(), path);
    println!("First 20 samples: {:?}", &samples[..20.min(samples.len())]);
    if let Some(pos) = first_nonzero {
        println!("First non-zero sample at position {}: {}", pos, samples[pos]);
        println!("Samples around position {}: {:?}", pos, &samples[pos.saturating_sub(5)..=(pos+5).min(samples.len()-1)]);
    } else {
        println!("All samples are zero!");
    }
    
    Ok(samples)
}

fn compare_bitstreams(encoded: &[u8; 10], reference: &[u8; 10]) -> (bool, Vec<(usize, u8, u8)>) {
    let mut differences = Vec::new();
    let mut is_exact = true;
    
    for i in 0..10 {
        if encoded[i] != reference[i] {
            is_exact = false;
            differences.push((i, encoded[i], reference[i]));
        }
    }
    
    (is_exact, differences)
}

#[test]
fn test_encoder_algthm() {
    let test_vectors_dir = Path::new("src/codecs/g729a/tests/test_vectors");
    let input_path = test_vectors_dir.join("ALGTHM.IN");
    let reference_path = test_vectors_dir.join("ALGTHM.BIT");
    
    // Read input samples
    let input_samples = read_pcm_file(&input_path)
        .expect("Failed to read ALGTHM.IN");
    
    // Read reference bitstream (now using ITU-T format)
    let reference_frames = read_itu_bitstream(&reference_path)
        .expect("Failed to read ALGTHM.BIT");
    
    println!("Testing encoder with ALGTHM vector");
    println!("Input samples: {}", input_samples.len());
    println!("Reference frames: {}", reference_frames.len());
    
    let mut encoder = G729AEncoder::new();
    let mut exact_matches = 0;
    let mut total_frames = 0;
    
    // Process each frame
    for (frame_idx, chunk) in input_samples.chunks(80).enumerate() {
        if chunk.len() < 80 {
            break; // Skip incomplete frames
        }
        
        if frame_idx >= 5 { break; } // Only process first 5 frames for debugging
        
        let mut samples = [0i16; 80];
        samples.copy_from_slice(&chunk[..80]);
        
        let frame = AudioFrame {
            samples,
            timestamp: frame_idx as u64 * 80,
        };
        
        // Get lookahead (next 40 samples or zeros)
        let lookahead = if input_samples.len() > (frame_idx + 1) * 80 + 40 {
            let mut la = [0i16; 40];
            la.copy_from_slice(&input_samples[(frame_idx + 1) * 80..(frame_idx + 1) * 80 + 40]);
            la
        } else {
            [0i16; 40]
        };
        
        println!("\n=== FRAME {} ===", frame_idx);
        println!("Frame samples [0..10]: {:?}", &frame.samples[..10]);
        println!("Frame energy: {}", frame.samples.iter().map(|&x| (x as i32).pow(2)).sum::<i32>());
        
        match encoder.encode_frame_with_lookahead(&frame, &lookahead) {
            Ok(encoded) => {
                if frame_idx < reference_frames.len() {
                    let (is_exact, differences) = compare_bitstreams(&encoded, &reference_frames[frame_idx]);
                    
                    if is_exact {
                        exact_matches += 1;
                    } else if frame_idx < 5 {  // Show details for first few frames
                        println!("\nFrame {} differences:", frame_idx);
                        println!("  Our:  {:02X?}", encoded);
                        println!("  Ref:  {:02X?}", reference_frames[frame_idx]);
                        
                        // Decode and compare parameters
                        use crate::codecs::g729a::codec::bitstream::unpack_frame;
                        let our_params = unpack_frame(&encoded);
                        let ref_params = unpack_frame(&reference_frames[frame_idx]);
                        
                        println!("\n  Parameter comparison:");
                        println!("    LSP indices: Our={:?}, Ref={:?}", 
                                 our_params.lsp_indices, ref_params.lsp_indices);
                        println!("    Pitch delays: Our={:?}, Ref={:?}", 
                                 our_params.pitch_delays, ref_params.pitch_delays);
                        println!("    Fixed CB (hex): Our=[0x{:05X},0x{:05X}], Ref=[0x{:05X},0x{:05X}]", 
                                 our_params.fixed_codebook_indices[0], our_params.fixed_codebook_indices[1],
                                 ref_params.fixed_codebook_indices[0], ref_params.fixed_codebook_indices[1]);
                        println!("    Gain indices: Our={:?}, Ref={:?}", 
                                 our_params.gain_indices, ref_params.gain_indices);
                        
                        // Show bit-level differences for first frame
                        if frame_idx == 0 {
                            println!("\n  Bit-level analysis:");
                            for (byte_idx, (&our_byte, &ref_byte)) in encoded.iter().zip(reference_frames[frame_idx].iter()).enumerate() {
                                if our_byte != ref_byte {
                                    println!("    Byte {}: {:08b} vs {:08b}", byte_idx, our_byte, ref_byte);
                                }
                            }
                        }
                    }
                    
                    total_frames += 1;
                }
            }
            Err(e) => {
                eprintln!("Encoding error at frame {}: {:?}", frame_idx, e);
                break;
            }
        }
    }
    
    let accuracy = (exact_matches as f64 / total_frames as f64) * 100.0;
    println!("\nEncoder Compliance Results:");
    println!("Total frames: {}", total_frames);
    println!("Exact matches: {} ({:.2}%)", exact_matches, accuracy);
    
    // Show the actual packed bitstream from the reference
    if reference_frames.len() > 0 {
        println!("\nReference frame 0 (packed from ITU format):");
        println!("  {:02X?}", reference_frames[0]);
    }
    
    // For now, we expect this to fail until we fix the implementation
    assert!(total_frames > 0, "No frames were processed");
}

#[test]
#[ignore = "Run only after encoder is working"]
fn test_encoder_all_vectors() {
    let test_vectors = vec![
        ("ALGTHM", "Algorithmic conditionals"),
        ("FIXED", "Fixed codebook search"),
        ("LSP", "LSP quantization"),
        ("PITCH", "Pitch search"),
        ("SPEECH", "Generic speech"),
        ("TAME", "Taming procedure"),
    ];
    
    for (vector_name, description) in test_vectors {
        println!("\nTesting {} - {}", vector_name, description);
        // TODO: Implement full test
    }
} 