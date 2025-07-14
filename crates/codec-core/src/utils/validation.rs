//! Input validation utilities for codec operations

use crate::error::{CodecError, Result};
use crate::types::{CodecType, SampleRate};

/// Validate audio samples for codec processing
pub fn validate_samples(samples: &[i16]) -> Result<()> {
    if samples.is_empty() {
        return Err(CodecError::invalid_format("Input samples cannot be empty"));
    }
    
    // Check for reasonable sample range
    for (i, &sample) in samples.iter().enumerate() {
        if sample.abs() > 32767 {
            return Err(CodecError::invalid_format(format!(
                "Sample at index {} out of range: {}",
                i, sample
            )));
        }
    }
    
    Ok(())
}

/// Validate encoded data for codec processing
pub fn validate_encoded_data(data: &[u8]) -> Result<()> {
    if data.is_empty() {
        return Err(CodecError::invalid_format("Encoded data cannot be empty"));
    }
    
    // Check for reasonable data size (not too large)
    if data.len() > 1024 * 1024 {
        return Err(CodecError::invalid_format(format!(
            "Encoded data too large: {} bytes",
            data.len()
        )));
    }
    
    Ok(())
}

/// Validate frame size for a specific codec
pub fn validate_frame_size(codec_type: CodecType, frame_size: usize) -> Result<()> {
    let expected_sizes = match codec_type {
        CodecType::G711Pcmu | CodecType::G711Pcma => {
            // G.711 commonly uses 10ms or 20ms frames at 8kHz
            vec![80, 160, 240, 320]
        }
        CodecType::G722 => {
            // G.722 commonly uses 10ms or 20ms frames at 16kHz
            vec![160, 320, 480, 640]
        }
        CodecType::G729 => {
            // G.729 uses fixed 10ms frames at 8kHz
            vec![80]
        }
        CodecType::Opus => {
            // Opus supports various frame sizes
            vec![120, 240, 480, 960, 1920, 2880]
        }
    };
    
    if !expected_sizes.contains(&frame_size) {
        return Err(CodecError::InvalidFrameSize {
            expected: expected_sizes[0],
            actual: frame_size,
        });
    }
    
    Ok(())
}

/// Validate sample rate for a specific codec
pub fn validate_sample_rate(codec_type: CodecType, sample_rate: SampleRate) -> Result<()> {
    let supported_rates = codec_type.supported_sample_rates();
    let rate_hz = sample_rate.hz();
    
    if !supported_rates.contains(&rate_hz) {
        return Err(CodecError::InvalidSampleRate {
            rate: rate_hz,
            supported: supported_rates.to_vec(),
        });
    }
    
    Ok(())
}

/// Validate channel count for a specific codec
pub fn validate_channels(codec_type: CodecType, channels: u8) -> Result<()> {
    let supported_channels = codec_type.supported_channels();
    
    if !supported_channels.contains(&channels) {
        return Err(CodecError::InvalidChannelCount {
            channels,
            supported: supported_channels.to_vec(),
        });
    }
    
    Ok(())
}

/// Validate bitrate for a specific codec
pub fn validate_bitrate(codec_type: CodecType, bitrate: u32) -> Result<()> {
    let (min_bitrate, max_bitrate) = codec_type.bitrate_range();
    
    if bitrate < min_bitrate || bitrate > max_bitrate {
        return Err(CodecError::InvalidBitrate {
            bitrate,
            min: min_bitrate,
            max: max_bitrate,
        });
    }
    
    Ok(())
}

/// Validate buffer sizes for encoding/decoding operations
pub fn validate_buffer_sizes(
    input_size: usize,
    output_size: usize,
    expected_ratio: f32,
) -> Result<()> {
    let expected_output_size = (input_size as f32 * expected_ratio) as usize;
    
    if output_size < expected_output_size {
        return Err(CodecError::BufferTooSmall {
            needed: expected_output_size,
            actual: output_size,
        });
    }
    
    Ok(())
}

/// Validate that frame samples are properly aligned for multi-channel audio
pub fn validate_channel_alignment(samples: &[i16], channels: u8) -> Result<()> {
    if samples.len() % channels as usize != 0 {
        return Err(CodecError::invalid_format(format!(
            "Sample count {} not divisible by channel count {}",
            samples.len(),
            channels
        )));
    }
    
    Ok(())
}

/// Validate G.711 specific parameters
pub fn validate_g711_frame(samples: &[i16], expected_frame_size: usize) -> Result<()> {
    validate_samples(samples)?;
    
    if samples.len() != expected_frame_size {
        return Err(CodecError::InvalidFrameSize {
            expected: expected_frame_size,
            actual: samples.len(),
        });
    }
    
    Ok(())
}

/// Validate G.722 specific parameters
pub fn validate_g722_frame(samples: &[i16], expected_frame_size: usize) -> Result<()> {
    validate_samples(samples)?;
    
    if samples.len() != expected_frame_size {
        return Err(CodecError::InvalidFrameSize {
            expected: expected_frame_size,
            actual: samples.len(),
        });
    }
    
    // G.722 requires even number of samples for QMF processing
    if samples.len() % 2 != 0 {
        return Err(CodecError::invalid_format(
            "G.722 requires even number of samples for QMF processing",
        ));
    }
    
    Ok(())
}

/// Validate G.729 specific parameters
pub fn validate_g729_frame(samples: &[i16]) -> Result<()> {
    validate_samples(samples)?;
    
    // G.729 uses fixed 80-sample frames (10ms at 8kHz)
    if samples.len() != 80 {
        return Err(CodecError::InvalidFrameSize {
            expected: 80,
            actual: samples.len(),
        });
    }
    
    Ok(())
}

/// Validate Opus specific parameters
pub fn validate_opus_frame(samples: &[i16], sample_rate: SampleRate) -> Result<()> {
    validate_samples(samples)?;
    
    let rate_hz = sample_rate.hz();
    let frame_size = samples.len();
    
    // Opus supports specific frame sizes based on sample rate
    let valid_frame_sizes = match rate_hz {
        8000 => vec![20, 40, 80, 160, 320, 480],
        12000 => vec![30, 60, 120, 240, 480, 720],
        16000 => vec![40, 80, 160, 320, 640, 960],
        24000 => vec![60, 120, 240, 480, 960, 1440],
        48000 => vec![120, 240, 480, 960, 1920, 2880],
        _ => return Err(CodecError::InvalidSampleRate {
            rate: rate_hz,
            supported: vec![8000, 12000, 16000, 24000, 48000],
        }),
    };
    
    if !valid_frame_sizes.contains(&frame_size) {
        return Err(CodecError::InvalidFrameSize {
            expected: valid_frame_sizes[0],
            actual: frame_size,
        });
    }
    
    Ok(())
}

/// Validate that two buffers have compatible sizes for processing
pub fn validate_buffer_compatibility(
    input: &[i16],
    output: &[u8],
    compression_ratio: f32,
) -> Result<()> {
    let expected_output_size = (input.len() as f32 * compression_ratio) as usize;
    
    if output.len() < expected_output_size {
        return Err(CodecError::BufferTooSmall {
            needed: expected_output_size,
            actual: output.len(),
        });
    }
    
    Ok(())
}

/// Validate memory alignment for SIMD operations
pub fn validate_simd_alignment(data: &[i16]) -> Result<()> {
    let ptr = data.as_ptr() as usize;
    
    // Check for 16-byte alignment (required for SSE)
    if ptr % 16 != 0 {
        tracing::debug!("Data not aligned for SIMD operations, falling back to scalar");
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SampleRate;

    #[test]
    fn test_validate_samples() {
        let valid_samples = vec![0, 1000, -1000, 16000, -16000];
        assert!(validate_samples(&valid_samples).is_ok());
        
        let empty_samples: Vec<i16> = vec![];
        assert!(validate_samples(&empty_samples).is_err());
    }

    #[test]
    fn test_validate_encoded_data() {
        let valid_data = vec![0u8, 127, 255, 64, 192];
        assert!(validate_encoded_data(&valid_data).is_ok());
        
        let empty_data: Vec<u8> = vec![];
        assert!(validate_encoded_data(&empty_data).is_err());
        
        let too_large_data = vec![0u8; 2 * 1024 * 1024]; // 2MB
        assert!(validate_encoded_data(&too_large_data).is_err());
    }

    #[test]
    fn test_validate_frame_size() {
        // G.711 valid frame sizes
        assert!(validate_frame_size(CodecType::G711Pcmu, 160).is_ok());
        assert!(validate_frame_size(CodecType::G711Pcmu, 123).is_err());
        
        // G.729 fixed frame size
        assert!(validate_frame_size(CodecType::G729, 80).is_ok());
        assert!(validate_frame_size(CodecType::G729, 160).is_err());
    }

    #[test]
    fn test_validate_sample_rate() {
        // G.711 supports only 8kHz
        assert!(validate_sample_rate(CodecType::G711Pcmu, SampleRate::Rate8000).is_ok());
        assert!(validate_sample_rate(CodecType::G711Pcmu, SampleRate::Rate48000).is_err());
        
        // Opus supports multiple rates
        assert!(validate_sample_rate(CodecType::Opus, SampleRate::Rate8000).is_ok());
        assert!(validate_sample_rate(CodecType::Opus, SampleRate::Rate48000).is_ok());
    }

    #[test]
    fn test_validate_channels() {
        // G.711 supports only mono
        assert!(validate_channels(CodecType::G711Pcmu, 1).is_ok());
        assert!(validate_channels(CodecType::G711Pcmu, 2).is_err());
        
        // Opus supports mono and stereo
        assert!(validate_channels(CodecType::Opus, 1).is_ok());
        assert!(validate_channels(CodecType::Opus, 2).is_ok());
        assert!(validate_channels(CodecType::Opus, 3).is_err());
    }

    #[test]
    fn test_validate_bitrate() {
        // G.711 has fixed bitrate
        assert!(validate_bitrate(CodecType::G711Pcmu, 64000).is_ok());
        assert!(validate_bitrate(CodecType::G711Pcmu, 128000).is_err());
        
        // Opus has variable bitrate
        assert!(validate_bitrate(CodecType::Opus, 32000).is_ok());
        assert!(validate_bitrate(CodecType::Opus, 600000).is_err());
    }

    #[test]
    fn test_validate_buffer_sizes() {
        // G.711 has 1:1 compression ratio (16-bit to 8-bit)
        assert!(validate_buffer_sizes(160, 80, 0.5).is_ok());
        assert!(validate_buffer_sizes(160, 40, 0.5).is_err());
    }

    #[test]
    fn test_validate_channel_alignment() {
        let mono_samples = vec![0, 1, 2, 3, 4]; // 5 samples
        assert!(validate_channel_alignment(&mono_samples, 1).is_ok());
        assert!(validate_channel_alignment(&mono_samples, 2).is_err());
        
        let stereo_samples = vec![0, 1, 2, 3]; // 4 samples = 2 stereo pairs
        assert!(validate_channel_alignment(&stereo_samples, 2).is_ok());
    }

    #[test]
    fn test_codec_specific_validation() {
        // G.711 frame validation
        let g711_frame = vec![0i16; 160];
        assert!(validate_g711_frame(&g711_frame, 160).is_ok());
        assert!(validate_g711_frame(&g711_frame, 80).is_err());
        
        // G.729 frame validation
        let g729_frame = vec![0i16; 80];
        assert!(validate_g729_frame(&g729_frame).is_ok());
        
        let wrong_g729_frame = vec![0i16; 160];
        assert!(validate_g729_frame(&wrong_g729_frame).is_err());
    }

    #[test]
    fn test_buffer_compatibility() {
        let input = vec![0i16; 160];
        let output = vec![0u8; 80];
        
        // G.711 compression ratio: 0.5 (16-bit to 8-bit)
        assert!(validate_buffer_compatibility(&input, &output, 0.5).is_ok());
        
        let small_output = vec![0u8; 40];
        assert!(validate_buffer_compatibility(&input, &small_output, 0.5).is_err());
    }
} 