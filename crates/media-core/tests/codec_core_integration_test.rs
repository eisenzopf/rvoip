//! Integration tests for codec-core integration

use rvoip_media_core::codec::factory::CodecFactory;
use rvoip_media_core::codec::audio::G711Codec;
use rvoip_media_core::codec::audio::common::AudioCodec;
use rvoip_media_core::types::AudioFrame;

#[test]
fn test_codec_factory_creates_correct_codecs() {
    // Test PCMU creation
    let pcmu_codec = CodecFactory::create_codec_default(0).unwrap();
    let info = pcmu_codec.get_info();
    assert!(info.name.contains("μ-law"));
    assert_eq!(info.sample_rate, 8000);
    assert_eq!(info.channels, 1);
    assert_eq!(info.bitrate, 64000); // 8000 * 8 * 1
    
    // Test PCMA creation
    let pcma_codec = CodecFactory::create_codec_default(8).unwrap();
    let info = pcma_codec.get_info();
    assert!(info.name.contains("A-law"));
    assert_eq!(info.sample_rate, 8000);
    assert_eq!(info.channels, 1);
    assert_eq!(info.bitrate, 64000); // 8000 * 8 * 1
}

#[test]
fn test_codec_factory_with_custom_params() {
    // Test with custom sample rate and channels
    let codec = CodecFactory::create_codec(0, Some(16000), Some(2)).unwrap();
    let info = codec.get_info();
    assert!(info.name.contains("μ-law"));
    assert_eq!(info.sample_rate, 16000);
    assert_eq!(info.channels, 2);
    assert_eq!(info.bitrate, 256000); // 16000 * 8 * 2
}

#[test]
fn test_g711_encoding_decoding() {
    let mut mu_codec = G711Codec::mu_law(8000, 1).unwrap();
    let mut a_codec = G711Codec::a_law(8000, 1).unwrap();
    
    // Test various sample patterns
    let test_patterns = vec![
        vec![0i16; 160],                              // Silence
        vec![1000i16; 160],                           // Constant tone
        vec![-1000i16; 160],                          // Negative constant tone
        (0..160).map(|i| (i * 100) as i16).collect(), // Linear ramp
        vec![i16::MAX; 160],                          // Maximum values
        vec![i16::MIN; 160],                          // Minimum values
    ];
    
    for samples in test_patterns {
        // Test μ-law
        let frame = AudioFrame::new(samples.clone(), 8000, 1, 0);
        let mu_encoded = mu_codec.encode(&frame).unwrap();
        assert_eq!(mu_encoded.len(), 160); // G.711 produces 1 byte per sample
        
        let mu_decoded = mu_codec.decode(&mu_encoded).unwrap();
        assert_eq!(mu_decoded.samples.len(), 160);
        assert_eq!(mu_decoded.sample_rate, 8000);
        assert_eq!(mu_decoded.channels, 1);
        
        // Test A-law
        let a_encoded = a_codec.encode(&frame).unwrap();
        assert_eq!(a_encoded.len(), 160);
        
        let a_decoded = a_codec.decode(&a_encoded).unwrap();
        assert_eq!(a_decoded.samples.len(), 160);
        assert_eq!(a_decoded.sample_rate, 8000);
        assert_eq!(a_decoded.channels, 1);
    }
}

#[test]
fn test_zero_copy_methods() {
    let mut codec = G711Codec::mu_law(8000, 1).unwrap();
    
    // Test zero-copy encoding
    let samples = vec![500i16; 160];
    let mut encoded_buffer = vec![0u8; 160];
    let encoded_len = codec.encode_to_buffer(&samples, &mut encoded_buffer).unwrap();
    assert_eq!(encoded_len, 160);
    
    // Test zero-copy decoding
    let mut decoded_buffer = vec![0i16; 160];
    let decoded_len = codec.decode_to_buffer(&encoded_buffer, &mut decoded_buffer).unwrap();
    assert_eq!(decoded_len, 160);
    
    // Verify the decoded samples are reasonable (G.711 has quantization)
    // We don't expect exact match due to lossy compression
    for sample in &decoded_buffer {
        assert!(sample.abs() < 1000); // Should be in reasonable range
    }
}

#[test]
fn test_error_handling() {
    let mut codec = G711Codec::mu_law(8000, 1).unwrap();
    
    // Test empty decode - codec-core may handle this gracefully
    let result = codec.decode(&[]);
    if result.is_err() {
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("G.711"));
        assert!(err_msg.contains("μ-law"));
    } else {
        // codec-core handles empty input gracefully, returning empty output
        let decoded = result.unwrap();
        assert_eq!(decoded.samples.len(), 0);
    }
    
    // Test insufficient output buffer for zero-copy
    let samples = vec![0i16; 160];
    let mut small_buffer = vec![0u8; 10]; // Too small
    let result = codec.encode_to_buffer(&samples, &mut small_buffer);
    assert!(result.is_err());
}

#[test]
fn test_codec_reset() {
    let mut codec = G711Codec::mu_law(8000, 1).unwrap();
    
    // Encode some data
    let frame = AudioFrame::new(vec![1000i16; 160], 8000, 1, 0);
    let _ = codec.encode(&frame).unwrap();
    
    // Reset codec (G.711 is stateless, but this should work)
    codec.reset();
    
    // Codec should still work after reset
    let encoded = codec.encode(&frame).unwrap();
    assert_eq!(encoded.len(), 160);
}

#[test]
fn test_transcoding_integration() {
    use rvoip_media_core::codec::transcoding::Transcoder;
    use rvoip_media_core::processing::format::FormatConverter;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    
    // Create transcoder with codec-core codecs
    let format_converter = Arc::new(RwLock::new(FormatConverter::new()));
    let mut transcoder = Transcoder::new(format_converter);
    
    // Test μ-law to A-law transcoding
    let pcmu_data = vec![0xFF; 160]; // 20ms μ-law frame
    
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let pcma_data = runtime.block_on(async {
        transcoder.transcode(&pcmu_data, 0, 8).await.unwrap()
    });
    
    assert_eq!(pcma_data.len(), 160); // Same size for G.711 variants
    assert_ne!(pcma_data, pcmu_data); // Should be different encoding
}

