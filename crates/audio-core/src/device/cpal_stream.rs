//! CPAL audio stream implementation for real-time capture and playback

use crate::error::{AudioError, AudioResult};
use crate::types::{AudioFrame, AudioFormat};
use crate::format::FormatConverter;
use cpal::traits::{DeviceTrait, StreamTrait};
use tokio::sync::mpsc;
use std::sync::{Arc, Mutex};

/// Find the best supported configuration for a device
fn find_best_config(
    device: &cpal::Device,
    desired_format: &AudioFormat,
    is_input: bool,
) -> AudioResult<(cpal::SupportedStreamConfig, AudioFormat)> {
    let supported_configs: Vec<cpal::SupportedStreamConfigRange> = if is_input {
        device.supported_input_configs()
            .map_err(|e| AudioError::DeviceError {
                device: device.name().unwrap_or_default(),
                operation: "get configs".to_string(),
                reason: e.to_string(),
            })?
            .collect()
    } else {
        device.supported_output_configs()
            .map_err(|e| AudioError::DeviceError {
                device: device.name().unwrap_or_default(),
                operation: "get configs".to_string(),
                reason: e.to_string(),
            })?
            .collect()
    };
    
    let desired_rate = desired_format.sample_rate;
    let desired_channels = desired_format.channels;
    
    // First, try to find exact match
    for config in &supported_configs {
        if config.channels() == desired_channels &&
           config.min_sample_rate().0 <= desired_rate &&
           config.max_sample_rate().0 >= desired_rate {
            let hardware_format = AudioFormat::new(
                desired_rate,
                desired_channels,
                16,
                desired_format.frame_size_ms,
            );
            return Ok((config.with_sample_rate(cpal::SampleRate(desired_rate)), hardware_format));
        }
    }
    
    // If no exact match, find the best alternative
    if supported_configs.is_empty() {
        return Err(AudioError::DeviceError {
            device: device.name().unwrap_or_default(),
            operation: "find config".to_string(),
            reason: "No supported configurations".to_string(),
        });
    }
    
    // Pick the first config and adjust to the closest supported values
    let config = &supported_configs[0];
    let hardware_channels = config.channels();
    
    // Pick the closest sample rate
    let hardware_rate = if desired_rate < config.min_sample_rate().0 {
        config.min_sample_rate().0
    } else if desired_rate > config.max_sample_rate().0 {
        config.max_sample_rate().0
    } else {
        desired_rate
    };
    
    let hardware_format = AudioFormat::new(
        hardware_rate,
        hardware_channels,
        16,
        desired_format.frame_size_ms,
    );
    
    tracing::error!(
        "🎵 Hardware format: {}Hz {} ch (requested: {}Hz {} ch)",
        hardware_rate, hardware_channels,
        desired_rate, desired_channels
    );
    
    Ok((config.with_sample_rate(cpal::SampleRate(hardware_rate)), hardware_format))
}

/// Create and start an audio capture stream
#[cfg(feature = "device-cpal")]
pub fn create_capture_stream(
    device: &cpal::Device,
    desired_format: AudioFormat,
    frame_tx: mpsc::Sender<AudioFrame>,
) -> AudioResult<cpal::Stream> {
    // Find the best hardware configuration
    let (config, hardware_format) = find_best_config(device, &desired_format, true)?;
    
    // Create format converter if needed
    let converter = if hardware_format.is_compatible_with(&desired_format) {
        None
    } else {
        tracing::error!("📐 Creating format converter for capture: {} -> {}", 
            hardware_format.description(), 
            desired_format.description()
        );
        Some(Arc::new(Mutex::new(FormatConverter::new(hardware_format.clone(), desired_format.clone())?)))
    };
    
    let err_fn = |err| tracing::error!("Audio capture stream error: {}", err);
    
    // Calculate samples per frame for the hardware format
    let hw_samples_per_frame = hardware_format.samples_per_frame();
    let mut buffer = Vec::with_capacity(hw_samples_per_frame);
    let mut timestamp = 0u32;
    
    // Build the stream
    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            // Convert f32 samples to i16
            for &sample in data {
                let i16_sample = (sample * i16::MAX as f32) as i16;
                buffer.push(i16_sample);
                
                // When we have a full frame, send it
                if buffer.len() >= hw_samples_per_frame {
                    let frame_samples: Vec<i16> = buffer.drain(..hw_samples_per_frame).collect();
                    let hw_frame = AudioFrame::new(frame_samples, hardware_format.clone(), timestamp);
                    
                    // Convert format if needed
                    let output_frame = if let Some(ref converter) = converter {
                        match converter.lock().unwrap().convert_frame(&hw_frame) {
                            Ok(converted) => converted,
                            Err(e) => {
                                tracing::error!("Format conversion error: {}", e);
                                return;
                            }
                        }
                    } else {
                        hw_frame
                    };
                    
                    // Try to send, but don't block if receiver is full
                    let _ = frame_tx.try_send(output_frame);
                    
                    timestamp = timestamp.wrapping_add(hw_samples_per_frame as u32);
                }
            }
        },
        err_fn,
        None, // No timeout
    ).map_err(|e| AudioError::DeviceError {
        device: device.name().unwrap_or_default(),
        operation: "build stream".to_string(),
        reason: e.to_string(),
    })?;
    
    // Start the stream
    stream.play().map_err(|e| AudioError::DeviceError {
        device: device.name().unwrap_or_default(),
        operation: "start stream".to_string(),
        reason: e.to_string(),
    })?;
    
    Ok(stream)
}

/// Create and start an audio playback stream
#[cfg(feature = "device-cpal")]
pub fn create_playback_stream(
    device: &cpal::Device,
    desired_format: AudioFormat,
    mut frame_rx: mpsc::Receiver<AudioFrame>,
) -> AudioResult<cpal::Stream> {
    // Find the best hardware configuration
    let (config, hardware_format) = find_best_config(device, &desired_format, false)?;
    
    // Create format converter if needed
    let converter = if desired_format.is_compatible_with(&hardware_format) {
        None
    } else {
        tracing::error!("📐 Creating format converter for playback: {} -> {}", 
            desired_format.description(), 
            hardware_format.description()
        );
        Some(Arc::new(Mutex::new(FormatConverter::new(desired_format.clone(), hardware_format.clone())?)))
    };
    
    let err_fn = |err| tracing::error!("Audio playback stream error: {}", err);
    
    // Playback buffer to smooth out timing
    let mut playback_buffer = Vec::new();
    let hw_samples_per_frame = hardware_format.samples_per_frame();
    let silence_frame = vec![0f32; hw_samples_per_frame];
    
    // Build the stream
    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            // Try to get more frames if buffer is low
            while playback_buffer.len() < data.len() {
                match frame_rx.try_recv() {
                    Ok(input_frame) => {
                        // Convert format if needed
                        let hw_frame = if let Some(ref converter) = converter {
                            match converter.lock().unwrap().convert_frame(&input_frame) {
                                Ok(converted) => converted,
                                Err(e) => {
                                    tracing::error!("Format conversion error: {}", e);
                                    // Use silence on conversion error
                                    playback_buffer.extend_from_slice(&silence_frame);
                                    continue;
                                }
                            }
                        } else {
                            input_frame
                        };
                        
                        // Convert i16 to f32 and add to buffer
                        for &sample in &hw_frame.samples {
                            let f32_sample = sample as f32 / i16::MAX as f32;
                            playback_buffer.push(f32_sample);
                        }
                    }
                    Err(_) => {
                        // No frames available, add silence
                        playback_buffer.extend_from_slice(&silence_frame);
                    }
                }
            }
            
            // Fill output buffer
            for (out_sample, &buf_sample) in data.iter_mut().zip(playback_buffer.iter()) {
                *out_sample = buf_sample;
            }
            
            // Remove consumed samples
            playback_buffer.drain(..data.len().min(playback_buffer.len()));
        },
        err_fn,
        None, // No timeout
    ).map_err(|e| AudioError::DeviceError {
        device: device.name().unwrap_or_default(),
        operation: "build stream".to_string(),
        reason: e.to_string(),
    })?;
    
    // Start the stream
    stream.play().map_err(|e| AudioError::DeviceError {
        device: device.name().unwrap_or_default(),
        operation: "start stream".to_string(),
        reason: e.to_string(),
    })?;
    
    Ok(stream)
}