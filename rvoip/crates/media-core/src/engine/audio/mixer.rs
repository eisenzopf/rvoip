use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use bytes::Bytes;
use tracing::{debug, trace, warn};
use uuid::Uuid;

use crate::codec::audio::common::{AudioFormat, SampleFormat};
use crate::error::{Error, Result};

/// Audio mixer configuration
#[derive(Debug, Clone)]
pub struct MixerConfig {
    /// Output audio format
    pub format: AudioFormat,
    /// Maximum number of streams to mix
    pub max_streams: usize,
    /// Automatic gain control
    pub auto_gain: bool,
    /// Clipping protection
    pub clip_protection: bool,
}

impl Default for MixerConfig {
    fn default() -> Self {
        Self {
            format: AudioFormat::pcm_telephony(),
            max_streams: 10,
            auto_gain: true,
            clip_protection: true,
        }
    }
}

/// Mixer stream handle
pub struct MixerStream {
    /// Stream ID
    id: Uuid,
    /// Stream format
    format: AudioFormat,
    /// Stream gain (0.0-2.0)
    gain: f32,
    /// Stream muted state
    muted: bool,
}

impl MixerStream {
    /// Create a new mixer stream
    fn new(format: AudioFormat) -> Self {
        Self {
            id: Uuid::new_v4(),
            format,
            gain: 1.0,
            muted: false,
        }
    }
    
    /// Get the stream ID
    pub fn id(&self) -> Uuid {
        self.id
    }
    
    /// Get the stream format
    pub fn format(&self) -> AudioFormat {
        self.format
    }
    
    /// Get the stream gain
    pub fn gain(&self) -> f32 {
        self.gain
    }
    
    /// Check if the stream is muted
    pub fn is_muted(&self) -> bool {
        self.muted
    }
    
    /// Set the stream gain
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain.clamp(0.0, 2.0);
    }
    
    /// Set the stream muted state
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }
}

/// Audio mixer for combining multiple streams
pub struct AudioMixer {
    /// Mixer configuration
    config: MixerConfig,
    /// Streams being mixed
    streams: RwLock<HashMap<Uuid, Arc<Mutex<MixerStream>>>>,
    /// Stream data buffers
    buffers: RwLock<HashMap<Uuid, Bytes>>,
    /// Frame size in samples
    frame_samples: usize,
    /// Mix buffer for processing
    mix_buffer: Mutex<Vec<f32>>,
}

impl AudioMixer {
    /// Create a new audio mixer
    pub fn new(config: MixerConfig) -> Self {
        // Calculate frame size for 20ms at the configured sample rate
        let frame_samples = (config.format.sample_rate.as_hz() as usize * 20) / 1000;
        frame_samples *= config.format.channels.channel_count() as usize;
        
        // Create mix buffer (using f32 for mixing precision)
        let mix_buffer = vec![0.0f32; frame_samples];
        
        Self {
            config,
            streams: RwLock::new(HashMap::new()),
            buffers: RwLock::new(HashMap::new()),
            frame_samples,
            mix_buffer: Mutex::new(mix_buffer),
        }
    }
    
    /// Create a new mixer with default configuration
    pub fn new_default() -> Self {
        Self::new(MixerConfig::default())
    }
    
    /// Add a stream to the mixer
    pub fn add_stream(&self, format: AudioFormat) -> Result<Arc<Mutex<MixerStream>>> {
        let mut streams = self.streams.write().unwrap();
        
        // Check if we've reached the maximum number of streams
        if streams.len() >= self.config.max_streams {
            return Err(Error::LimitExceeded(
                format!("Maximum number of streams ({}) reached", self.config.max_streams)
            ));
        }
        
        // Create a new stream
        let stream = Arc::new(Mutex::new(MixerStream::new(format)));
        let id = stream.lock().unwrap().id();
        
        // Add to streams map
        streams.insert(id, stream.clone());
        
        // Add empty buffer
        let mut buffers = self.buffers.write().unwrap();
        buffers.insert(id, Bytes::new());
        
        debug!("Added stream to mixer: id={}", id);
        
        Ok(stream)
    }
    
    /// Remove a stream from the mixer
    pub fn remove_stream(&self, id: Uuid) -> Result<()> {
        let mut streams = self.streams.write().unwrap();
        
        if streams.remove(&id).is_some() {
            let mut buffers = self.buffers.write().unwrap();
            buffers.remove(&id);
            
            debug!("Removed stream from mixer: id={}", id);
            Ok(())
        } else {
            Err(Error::StreamNotFound(id.to_string()))
        }
    }
    
    /// Add audio data to a stream
    pub fn add_data(&self, id: Uuid, data: Bytes) -> Result<()> {
        // Check if stream exists
        let streams = self.streams.read().unwrap();
        if !streams.contains_key(&id) {
            return Err(Error::StreamNotFound(id.to_string()));
        }
        
        // Store data
        let mut buffers = self.buffers.write().unwrap();
        buffers.insert(id, data);
        
        Ok(())
    }
    
    /// Mix all streams and return the result
    pub fn mix(&self) -> Result<Bytes> {
        // Reset mix buffer
        let mut mix_buf = self.mix_buffer.lock().unwrap();
        for sample in mix_buf.iter_mut() {
            *sample = 0.0;
        }
        
        // Get streams and buffers
        let streams = self.streams.read().unwrap();
        let buffers = self.buffers.read().unwrap();
        
        // Collect active streams
        let mut active_streams = 0;
        
        // Mix each stream
        for (id, stream) in streams.iter() {
            let stream = stream.lock().unwrap();
            
            // Skip muted streams
            if stream.muted {
                continue;
            }
            
            // Get stream data
            if let Some(data) = buffers.get(id) {
                if !data.is_empty() {
                    // Convert to the mixer's format and add to mix buffer
                    self.add_to_mix(&stream, data, &mut mix_buf)?;
                    active_streams += 1;
                }
            }
        }
        
        // Apply automatic gain control if needed
        if self.config.auto_gain && active_streams > 1 {
            let gain_factor = 1.0 / (active_streams as f32).sqrt();
            
            for sample in mix_buf.iter_mut() {
                *sample *= gain_factor;
            }
        }
        
        // Apply clipping protection if needed
        if self.config.clip_protection {
            for sample in mix_buf.iter_mut() {
                // Soft clipping using tanh
                if sample.abs() > 0.9 {
                    *sample = sample.signum() * (0.9 + 0.1 * (((*sample).abs() - 0.9) * 10.0).tanh());
                }
            }
        }
        
        // Convert mixed buffer to output format
        let output = self.convert_to_output(&mix_buf)?;
        
        Ok(output)
    }
    
    /// Add stream data to the mix buffer
    fn add_to_mix(&self, stream: &MixerStream, data: &Bytes, mix_buf: &mut [f32]) -> Result<()> {
        let src_format = stream.format;
        let dst_format = self.config.format;
        
        // Check if formats match
        let format_matches = 
            src_format.sample_rate == dst_format.sample_rate &&
            src_format.channels.channel_count() == dst_format.channels.channel_count();
        
        // Calculate frame size in samples
        let samples_per_frame = self.frame_samples;
        
        // Apply gain
        let gain = stream.gain;
        
        match src_format.format {
            SampleFormat::S16 => {
                // Calculate number of samples to mix
                let src_samples = data.len() / 2;
                let samples_to_mix = src_samples.min(samples_per_frame);
                
                // Convert and mix
                for i in 0..samples_to_mix {
                    if i * 2 + 1 >= data.len() {
                        break;
                    }
                    
                    let sample = i16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                    let normalized = (sample as f32) / 32768.0;
                    
                    mix_buf[i] += normalized * gain;
                }
            },
            SampleFormat::F32 => {
                // Calculate number of samples to mix
                let src_samples = data.len() / 4;
                let samples_to_mix = src_samples.min(samples_per_frame);
                
                // Access as f32 slice
                let float_data = unsafe {
                    std::slice::from_raw_parts(
                        data.as_ptr() as *const f32,
                        src_samples
                    )
                };
                
                // Mix directly
                for i in 0..samples_to_mix {
                    if i >= float_data.len() {
                        break;
                    }
                    
                    mix_buf[i] += float_data[i] * gain;
                }
            },
            _ => {
                warn!("Unsupported sample format for mixing: {:?}", src_format.format);
                return Err(Error::UnsupportedFormat(
                    format!("Cannot mix {:?} format", src_format.format)
                ));
            }
        }
        
        Ok(())
    }
    
    /// Convert mixed buffer to output format
    fn convert_to_output(&self, mix_buf: &[f32]) -> Result<Bytes> {
        let format = self.config.format.format;
        let sample_count = mix_buf.len();
        
        match format {
            SampleFormat::S16 => {
                let mut output = vec![0u8; sample_count * 2];
                
                for i in 0..sample_count {
                    let sample = (mix_buf[i] * 32767.0).clamp(-32768.0, 32767.0) as i16;
                    let bytes = sample.to_le_bytes();
                    output[i * 2] = bytes[0];
                    output[i * 2 + 1] = bytes[1];
                }
                
                Ok(Bytes::from(output))
            },
            SampleFormat::F32 => {
                let mut output = vec![0u8; sample_count * 4];
                
                for i in 0..sample_count {
                    let sample = mix_buf[i].clamp(-1.0, 1.0);
                    let bytes = sample.to_le_bytes();
                    output[i * 4] = bytes[0];
                    output[i * 4 + 1] = bytes[1];
                    output[i * 4 + 2] = bytes[2];
                    output[i * 4 + 3] = bytes[3];
                }
                
                Ok(Bytes::from(output))
            },
            _ => {
                Err(Error::UnsupportedFormat(
                    format!("Cannot output to {:?} format", format)
                ))
            }
        }
    }
    
    /// Get the number of streams
    pub fn stream_count(&self) -> usize {
        let streams = self.streams.read().unwrap();
        streams.len()
    }
    
    /// Get all stream IDs
    pub fn stream_ids(&self) -> Vec<Uuid> {
        let streams = self.streams.read().unwrap();
        streams.keys().cloned().collect()
    }
    
    /// Get stream by ID
    pub fn get_stream(&self, id: Uuid) -> Option<Arc<Mutex<MixerStream>>> {
        let streams = self.streams.read().unwrap();
        streams.get(&id).cloned()
    }
    
    /// Get mixer configuration
    pub fn config(&self) -> &MixerConfig {
        &self.config
    }
} 