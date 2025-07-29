use g729a_new::decoder::G729ADecoder;
use g729a_new::common::tab_ld8a::L_FRAME;
use g729a_new::common::bits::SERIAL_SIZE;

#[test]
fn test_decoder_init() {
    let mut decoder = G729ADecoder::new();
    decoder.init();
    
    // Decoder should initialize without crashing
    // This is mainly a smoke test
}

#[test]
fn test_decoder_basic_frame() {
    let mut decoder = G729ADecoder::new();
    decoder.init();
    
    // Create a minimal valid bitstream
    let mut bitstream = [0i16; SERIAL_SIZE];
    bitstream[0] = 0x6b21; // Sync word
    bitstream[1] = 80;     // Frame size
    
    // Decode the frame
    let speech = decoder.decode_frame(&bitstream);
    
    // Verify output length
    assert_eq!(speech.len(), L_FRAME, "Decoded speech should be {} samples", L_FRAME);
    
    // Output should be reasonable values (basic sanity check)
    for sample in &speech {
        assert!(
            *sample >= -32768 && *sample <= 32767,
            "All samples should be within i16 range: {}",
            *sample
        );
    }
}

#[test]
fn test_decoder_bad_frame() {
    let mut decoder = G729ADecoder::new();
    decoder.init();
    
    // Create an invalid bitstream (wrong sync word)
    let mut bitstream = [0i16; SERIAL_SIZE];
    bitstream[0] = 0x1234; // Wrong sync word
    bitstream[1] = 80;
    
    // Decode the frame (should use error concealment)
    let speech = decoder.decode_frame(&bitstream);
    
    // Should still produce valid output
    assert_eq!(speech.len(), L_FRAME, "Error concealment should produce full frame");
}

#[test]
fn test_decoder_multiple_frames() {
    let mut decoder = G729ADecoder::new();
    decoder.init();
    
    // Create a valid bitstream
    let mut bitstream = [0i16; SERIAL_SIZE];
    bitstream[0] = 0x6b21; // Sync word
    bitstream[1] = 80;     // Frame size
    
    // Decode multiple frames to test state persistence
    let speech1 = decoder.decode_frame(&bitstream);
    let speech2 = decoder.decode_frame(&bitstream);
    let speech3 = decoder.decode_frame(&bitstream);
    
    // All frames should be valid
    assert_eq!(speech1.len(), L_FRAME);
    assert_eq!(speech2.len(), L_FRAME);
    assert_eq!(speech3.len(), L_FRAME);
    
    // Basic consistency check - decoder should produce valid output consistently
    // For the same input bitstream, we expect similar but possibly not identical output
    // due to internal state evolution (this is normal for stateful decoders)
}

#[test]
fn test_decoder_reset() {
    let mut decoder = G729ADecoder::new();
    decoder.init();
    
    // Create a valid bitstream
    let mut bitstream = [0i16; SERIAL_SIZE];
    bitstream[0] = 0x6b21;
    bitstream[1] = 80;
    
    // Decode a frame
    let speech1 = decoder.decode_frame(&bitstream);
    
    // Reset decoder
    decoder.init();
    
    // Decode the same frame again
    let speech2 = decoder.decode_frame(&bitstream);
    
    // Results should be identical after reset
    assert_eq!(speech1, speech2, "Decoder should produce identical output after reset");
}