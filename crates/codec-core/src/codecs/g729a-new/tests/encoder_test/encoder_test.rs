use std::fs::File;
use std::io::Read;

// Import G.729A encoder components
use g729a_new::common::basic_operators::Word16;
use g729a_new::encoder::g729a_encoder::{G729AEncoder, export_prm2bits as prm2bits};
use g729a_new::common::bits::{PRM_SIZE, SERIAL_SIZE};

const L_FRAME: usize = 80; // Frame size (10ms at 8kHz)

/// Test basic encoder functionality
#[test]
fn test_encoder_initialization() {
    let mut encoder = G729AEncoder::new();
    encoder.init();
    
    // Create a simple test frame (silence)
    let speech_frame = [0i16; L_FRAME];
    
    // Encode the frame
    let prm = encoder.encode_frame(&speech_frame);
    
    // Check that we got parameters
    assert_eq!(prm.len(), PRM_SIZE);
    
    // Convert to bitstream
    let serial = prm2bits(&prm);
    assert_eq!(serial.len(), SERIAL_SIZE);
}

/// Test encoder with sine wave input
#[test]
fn test_encoder_sine_wave() {
    let mut encoder = G729AEncoder::new();
    encoder.init();
    
    // Generate a 1kHz sine wave at 8kHz sampling rate
    let mut speech_frame = [0i16; L_FRAME];
    for i in 0..L_FRAME {
        let phase = 2.0 * std::f64::consts::PI * 1000.0 * (i as f64) / 8000.0;
        speech_frame[i] = (8000.0 * phase.sin()) as i16; // Amplitude ~8000
    }
    
    // Encode the frame
    let prm = encoder.encode_frame(&speech_frame);
    
    // Basic validation - LSP indices should be non-zero for voiced speech
    assert!(prm[0] != 0 || prm[1] != 0, "LSP indices should not be zero for voiced input");
}

/// Test encoder with test vector if available
#[test]
fn test_encoder_with_test_vector() {
    let test_vector_path = "/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/codec-core/src/codecs/g729a-new/test_vectors/SPEECH.IN";
    
    // Try to open test vector file
    let mut file = match File::open(test_vector_path) {
        Ok(f) => f,
        Err(_) => {
            println!("Test vector not found, skipping test");
            return;
        }
    };
    
    let mut encoder = G729AEncoder::new();
    encoder.init();
    
    let mut buffer = [0u8; L_FRAME * 2]; // 2 bytes per sample
    let mut frame_count = 0;
    
    // Process first few frames
    while frame_count < 10 && file.read_exact(&mut buffer).is_ok() {
        // Convert to Word16
        let mut speech_frame = [0i16; L_FRAME];
        for i in 0..L_FRAME {
            speech_frame[i] = i16::from_le_bytes([buffer[i*2], buffer[i*2+1]]);
        }
        
        // Encode
        let prm = encoder.encode_frame(&speech_frame);
        
        // Basic validation
        assert_eq!(prm.len(), PRM_SIZE);
        
        frame_count += 1;
    }
    
    println!("Successfully processed {} test frames", frame_count);
}

/// Test multiple frames to check state continuity
#[test]
fn test_encoder_state_continuity() {
    let mut encoder = G729AEncoder::new();
    encoder.init();
    
    // Process multiple frames
    for frame_idx in 0..5 {
        let mut speech_frame = [0i16; L_FRAME];
        
        // Create different patterns for each frame
        for i in 0..L_FRAME {
            speech_frame[i] = ((i + frame_idx * 100) % 1000) as i16;
        }
        
        let prm = encoder.encode_frame(&speech_frame);
        assert_eq!(prm.len(), PRM_SIZE);
        
        // Convert to bits to ensure bit packing works
        let serial = prm2bits(&prm);
        assert_eq!(serial.len(), SERIAL_SIZE);
    }
}