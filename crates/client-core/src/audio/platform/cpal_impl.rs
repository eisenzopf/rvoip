//! CPAL-based audio device implementation
//!
//! This module provides real hardware audio device support using the CPAL
//! (Cross-Platform Audio Library) crate for actual audio input/output.

#[cfg(feature = "audio-cpal")]
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, Host, Stream, StreamConfig, SupportedStreamConfig,
};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::audio::{
    AudioDevice, AudioDeviceInfo, AudioDirection, AudioError, AudioFormat, AudioResult,
    device::AudioFrame,
};

/// CPAL-based audio device implementation
#[cfg(feature = "audio-cpal")]
pub struct CpalAudioDevice {
    info: AudioDeviceInfo,
    device: Device,
    is_active: Arc<std::sync::atomic::AtomicBool>,
    current_format: Arc<Mutex<Option<AudioFormat>>>,
    // Use a shutdown channel instead of storing the stream
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

#[cfg(feature = "audio-cpal")]
impl std::fmt::Debug for CpalAudioDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpalAudioDevice")
            .field("info", &self.info)
            .field("is_active", &self.is_active.load(std::sync::atomic::Ordering::Relaxed))
            .field("current_format", &self.current_format)
            .finish()
    }
}

#[cfg(feature = "audio-cpal")]
impl CpalAudioDevice {
    /// Create a new CPAL audio device
    pub fn new(info: AudioDeviceInfo, device: Device) -> Self {
        Self {
            info,
            device,
            is_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            current_format: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Convert AudioFormat to CPAL StreamConfig
    fn audio_format_to_stream_config(&self, format: &AudioFormat) -> Result<StreamConfig, AudioError> {
        let config = StreamConfig {
            channels: format.channels,
            sample_rate: cpal::SampleRate(format.sample_rate),
            buffer_size: cpal::BufferSize::Fixed(
                (format.sample_rate * format.frame_size_ms / 1000) as u32
            ),
        };
        Ok(config)
    }

    /// Convert CPAL samples to AudioFrame
    fn samples_to_audio_frame(
        samples: &[f32],
        format: &AudioFormat,
        timestamp_ms: u64,
    ) -> AudioFrame {
        // Convert f32 samples to i16
        let i16_samples: Vec<i16> = samples
            .iter()
            .map(|&sample| (sample * i16::MAX as f32) as i16)
            .collect();

        AudioFrame::new(i16_samples, format.clone(), timestamp_ms)
    }

    /// Convert AudioFrame to CPAL samples
    fn audio_frame_to_samples(frame: &AudioFrame) -> Vec<f32> {
        frame
            .samples
            .iter()
            .map(|&sample| sample as f32 / i16::MAX as f32)
            .collect()
    }

    /// Get supported stream configuration for the device
    fn get_supported_config(&self, format: &AudioFormat) -> Result<SupportedStreamConfig, AudioError> {
        // Helper function to find matching config
        fn find_matching_config<I>(
            configs: I,
            format: &AudioFormat,
        ) -> Option<SupportedStreamConfig>
        where
            I: Iterator<Item = cpal::SupportedStreamConfigRange>,
        {
            for config in configs {
                if config.channels() == format.channels
                    && config.min_sample_rate().0 <= format.sample_rate
                    && config.max_sample_rate().0 >= format.sample_rate
                {
                    return Some(config.with_sample_rate(cpal::SampleRate(format.sample_rate)));
                }
            }
            None
        }

        let supported_config = match self.info.direction {
            AudioDirection::Input => {
                let configs = self.device.supported_input_configs()
                    .map_err(|e| AudioError::PlatformError {
                        message: format!("Failed to get supported input configs: {}", e),
                    })?;
                find_matching_config(configs, format)
            }
            AudioDirection::Output => {
                let configs = self.device.supported_output_configs()
                    .map_err(|e| AudioError::PlatformError {
                        message: format!("Failed to get supported output configs: {}", e),
                    })?;
                find_matching_config(configs, format)
            }
        };

        supported_config.ok_or_else(|| AudioError::FormatNotSupported {
            format: format.clone(),
            device_id: self.info.id.clone(),
        })
    }
}

#[cfg(feature = "audio-cpal")]
#[async_trait::async_trait]
impl AudioDevice for CpalAudioDevice {
    fn info(&self) -> &AudioDeviceInfo {
        &self.info
    }

    async fn start_capture(&self, format: AudioFormat) -> AudioResult<mpsc::Receiver<AudioFrame>> {
        if self.info.direction != AudioDirection::Input {
            return Err(AudioError::ConfigurationError {
                message: "Cannot capture on output device".to_string(),
            });
        }

        let (tx, rx) = mpsc::channel(4096);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        
        let supported_config = self.get_supported_config(&format)?;
        let stream_config = supported_config.config();

        let format_clone = format.clone();
        let device_name = self.info.name.clone();
        let is_active = self.is_active.clone();

        // Store shutdown sender
        {
            let mut shutdown_guard = self.shutdown_tx.lock().unwrap();
            *shutdown_guard = Some(shutdown_tx);
        }

        // Spawn thread to manage the stream (since CPAL streams are not Send)
        let device_for_task = self.device.clone();
        std::thread::spawn(move || {
            let stream = match device_for_task.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let timestamp_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;

                    let audio_frame =
                        Self::samples_to_audio_frame(data, &format_clone, timestamp_ms);

                    if let Err(e) = tx.try_send(audio_frame) {
                        warn!("Failed to send audio frame: {}", e);
                    }
                },
                |err| {
                    error!("Audio capture error: {}", err);
                },
                None,
            ) {
                Ok(stream) => stream,
                Err(e) => {
                    error!("Failed to build input stream: {}", e);
                    return;
                }
            };

            // Start the stream
            if let Err(e) = stream.play() {
                error!("Failed to start input stream: {}", e);
                return;
            }

            info!("Started audio capture on device: {}", device_name);

            // Wait for shutdown signal (use blocking wait since we're in a thread)
            match shutdown_rx.blocking_recv() {
                Ok(()) | Err(_) => {
                    // Stream is automatically dropped here, stopping it
                    is_active.store(false, std::sync::atomic::Ordering::Relaxed);
                    info!("Stopped audio capture on device: {}", device_name);
                }
            }
        });

        // Update state
        self.is_active.store(true, std::sync::atomic::Ordering::Relaxed);
        {
            let mut format_guard = self.current_format.lock().unwrap();
            *format_guard = Some(format);
        }

        Ok(rx)
    }

    async fn stop_capture(&self) -> AudioResult<()> {
        // Send shutdown signal if active
        if self.is_active.load(std::sync::atomic::Ordering::Relaxed) {
            let mut shutdown_guard = self.shutdown_tx.lock().unwrap();
            if let Some(shutdown_tx) = shutdown_guard.take() {
                let _ = shutdown_tx.send(()); // Signal shutdown
            }
        }

        // Clear format
        {
            let mut format_guard = self.current_format.lock().unwrap();
            *format_guard = None;
        }

        Ok(())
    }

    async fn start_playback(&self, format: AudioFormat) -> AudioResult<mpsc::Sender<AudioFrame>> {
        if self.info.direction != AudioDirection::Output {
            return Err(AudioError::ConfigurationError {
                message: "Cannot playback on input device".to_string(),
            });
        }

        let (tx, mut rx) = mpsc::channel::<AudioFrame>(4096);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        
        let supported_config = self.get_supported_config(&format)?;
        let stream_config = supported_config.config();

        let device_name = self.info.name.clone();
        let is_active = self.is_active.clone();

        // Store shutdown sender
        {
            let mut shutdown_guard = self.shutdown_tx.lock().unwrap();
            *shutdown_guard = Some(shutdown_tx);
        }

        // Spawn thread to manage the stream (since CPAL streams are not Send)
        let device_for_task = self.device.clone();
        std::thread::spawn(move || {
            // Create tokio runtime for this thread
            let rt = tokio::runtime::Runtime::new().unwrap();
            
            // Shared buffer for audio data
            let audio_buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
            let audio_buffer_clone = audio_buffer.clone();

            // Spawn task to receive audio frames and buffer them
            let _handle = rt.spawn(async move {
                while let Some(frame) = rx.recv().await {
                    let samples = Self::audio_frame_to_samples(&frame);
                    let mut buffer = audio_buffer_clone.lock().unwrap();
                    buffer.extend_from_slice(&samples);
                }
            });

            // Create output stream
            let stream = match device_for_task.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut buffer = audio_buffer.lock().unwrap();
                    
                    // Fill output buffer with available audio data
                    let available = buffer.len().min(data.len());
                    if available > 0 {
                        data[..available].copy_from_slice(&buffer[..available]);
                        buffer.drain(..available);
                        
                        // Fill remaining with silence if needed
                        if available < data.len() {
                            data[available..].fill(0.0);
                        }
                    } else {
                        // No audio data available, output silence
                        data.fill(0.0);
                    }
                },
                |err| {
                    error!("Audio playback error: {}", err);
                },
                None,
            ) {
                Ok(stream) => stream,
                Err(e) => {
                    error!("Failed to build output stream: {}", e);
                    return;
                }
            };

            // Start the stream
            if let Err(e) = stream.play() {
                error!("Failed to start output stream: {}", e);
                return;
            }

            info!("Started audio playback on device: {}", device_name);

            // Wait for shutdown signal (use blocking wait since we're in a thread)
            match shutdown_rx.blocking_recv() {
                Ok(()) | Err(_) => {
                    // Stream is automatically dropped here, stopping it
                    is_active.store(false, std::sync::atomic::Ordering::Relaxed);
                    info!("Stopped audio playback on device: {}", device_name);
                }
            }
        });

        // Update state
        self.is_active.store(true, std::sync::atomic::Ordering::Relaxed);
        {
            let mut format_guard = self.current_format.lock().unwrap();
            *format_guard = Some(format);
        }

        Ok(tx)
    }

    async fn stop_playback(&self) -> AudioResult<()> {
        // Send shutdown signal if active
        if self.is_active.load(std::sync::atomic::Ordering::Relaxed) {
            let mut shutdown_guard = self.shutdown_tx.lock().unwrap();
            if let Some(shutdown_tx) = shutdown_guard.take() {
                let _ = shutdown_tx.send(()); // Signal shutdown
            }
        }

        // Clear format
        {
            let mut format_guard = self.current_format.lock().unwrap();
            *format_guard = None;
        }

        Ok(())
    }

    fn is_active(&self) -> bool {
        self.is_active.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn current_format(&self) -> Option<AudioFormat> {
        let format_guard = self.current_format.lock().unwrap();
        format_guard.clone()
    }
}

/// CPAL platform implementation
#[cfg(feature = "audio-cpal")]
pub struct CpalPlatform {
    host: Host,
}

#[cfg(feature = "audio-cpal")]
impl CpalPlatform {
    /// Create a new CPAL platform instance
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }

    /// List all available audio devices
    pub fn list_devices(&self, direction: AudioDirection) -> AudioResult<Vec<AudioDeviceInfo>> {
        let devices = match direction {
            AudioDirection::Input => self.host.input_devices(),
            AudioDirection::Output => self.host.output_devices(),
        }
        .map_err(|e| AudioError::PlatformError {
            message: format!("Failed to enumerate {} devices: {}", 
                           match direction { AudioDirection::Input => "input", AudioDirection::Output => "output" }, e),
        })?;

        let mut device_infos = Vec::new();
        
        for (idx, device) in devices.enumerate() {
            let name = device.name().unwrap_or_else(|_| format!("Unknown Device {}", idx));
            
            // Get supported sample rates and channels
            let (supported_sample_rates, supported_channels) = match direction {
                AudioDirection::Input => {
                    match device.supported_input_configs() {
                        Ok(configs) => {
                            let mut sample_rates = Vec::new();
                            let mut channels = Vec::new();
                            
                            for config in configs {
                                sample_rates.push(config.min_sample_rate().0);
                                sample_rates.push(config.max_sample_rate().0);
                                channels.push(config.channels());
                            }
                            
                            sample_rates.sort_unstable();
                            sample_rates.dedup();
                            channels.sort_unstable();
                            channels.dedup();
                            
                            (sample_rates, channels)
                        }
                        Err(_) => {
                            // Fallback to common values
                            (vec![8000, 16000, 44100, 48000], vec![1, 2])
                        }
                    }
                }
                AudioDirection::Output => {
                    match device.supported_output_configs() {
                        Ok(configs) => {
                            let mut sample_rates = Vec::new();
                            let mut channels = Vec::new();
                            
                            for config in configs {
                                sample_rates.push(config.min_sample_rate().0);
                                sample_rates.push(config.max_sample_rate().0);
                                channels.push(config.channels());
                            }
                            
                            sample_rates.sort_unstable();
                            sample_rates.dedup();
                            channels.sort_unstable();
                            channels.dedup();
                            
                            (sample_rates, channels)
                        }
                        Err(_) => {
                            // Fallback to common values
                            (vec![8000, 16000, 44100, 48000], vec![1, 2])
                        }
                    }
                }
            };

            let device_info = AudioDeviceInfo {
                id: format!("cpal-{}-{}", 
                          match direction { AudioDirection::Input => "input", AudioDirection::Output => "output" }, 
                          idx),
                name,
                direction,
                is_default: false, // We'll set this later for default devices
                supported_sample_rates,
                supported_channels,
            };
            
            device_infos.push(device_info);
        }

        Ok(device_infos)
    }

    /// Get the default audio device for the given direction
    pub fn get_default_device(&self, direction: AudioDirection) -> AudioResult<Arc<dyn AudioDevice>> {
        let device = match direction {
            AudioDirection::Input => self.host.default_input_device(),
            AudioDirection::Output => self.host.default_output_device(),
        }
        .ok_or_else(|| AudioError::DeviceNotFound {
            device_id: format!("default-{}", 
                              match direction { AudioDirection::Input => "input", AudioDirection::Output => "output" }),
        })?;

        let name = device.name().unwrap_or_else(|_| "Default Device".to_string());
        
        // Get supported formats
        let (supported_sample_rates, supported_channels) = match direction {
            AudioDirection::Input => {
                match device.supported_input_configs() {
                    Ok(configs) => {
                        let mut sample_rates = Vec::new();
                        let mut channels = Vec::new();
                        
                        for config in configs {
                            sample_rates.push(config.min_sample_rate().0);
                            sample_rates.push(config.max_sample_rate().0);
                            channels.push(config.channels());
                        }
                        
                        sample_rates.sort_unstable();
                        sample_rates.dedup();
                        channels.sort_unstable();
                        channels.dedup();
                        
                        (sample_rates, channels)
                    }
                    Err(_) => {
                        // Fallback to common values
                        (vec![8000, 16000, 44100, 48000], vec![1, 2])
                    }
                }
            }
            AudioDirection::Output => {
                match device.supported_output_configs() {
                    Ok(configs) => {
                        let mut sample_rates = Vec::new();
                        let mut channels = Vec::new();
                        
                        for config in configs {
                            sample_rates.push(config.min_sample_rate().0);
                            sample_rates.push(config.max_sample_rate().0);
                            channels.push(config.channels());
                        }
                        
                        sample_rates.sort_unstable();
                        sample_rates.dedup();
                        channels.sort_unstable();
                        channels.dedup();
                        
                        (sample_rates, channels)
                    }
                    Err(_) => {
                        // Fallback to common values
                        (vec![8000, 16000, 44100, 48000], vec![1, 2])
                    }
                }
            }
        };

        let device_info = AudioDeviceInfo {
            id: format!("cpal-default-{}", 
                       match direction { AudioDirection::Input => "input", AudioDirection::Output => "output" }),
            name,
            direction,
            is_default: true,
            supported_sample_rates,
            supported_channels,
        };

        Ok(Arc::new(CpalAudioDevice::new(device_info, device)))
    }

    /// Create a specific device by ID
    pub fn create_device(&self, device_id: &str) -> AudioResult<Arc<dyn AudioDevice>> {
        if device_id.starts_with("cpal-default-") {
            let direction = if device_id.contains("input") {
                AudioDirection::Input
            } else {
                AudioDirection::Output
            };
            return self.get_default_device(direction);
        }

        // Parse device ID to get direction and index
        let parts: Vec<&str> = device_id.split('-').collect();
        if parts.len() != 3 || parts[0] != "cpal" {
            return Err(AudioError::DeviceNotFound {
                device_id: device_id.to_string(),
            });
        }

        let direction = match parts[1] {
            "input" => AudioDirection::Input,
            "output" => AudioDirection::Output,
            _ => return Err(AudioError::DeviceNotFound {
                device_id: device_id.to_string(),
            }),
        };

        let index: usize = parts[2].parse().map_err(|_| AudioError::DeviceNotFound {
            device_id: device_id.to_string(),
        })?;

        // Get the device by index
        let mut devices = match direction {
            AudioDirection::Input => self.host.input_devices(),
            AudioDirection::Output => self.host.output_devices(),
        }
        .map_err(|e| AudioError::PlatformError {
            message: format!("Failed to enumerate devices: {}", e),
        })?;

        let device = devices
            .nth(index)
            .ok_or_else(|| AudioError::DeviceNotFound {
                device_id: device_id.to_string(),
            })?;

        let name = device.name().unwrap_or_else(|_| format!("Device {}", index));
        
        let device_info = AudioDeviceInfo {
            id: device_id.to_string(),
            name,
            direction,
            is_default: false,
            supported_sample_rates: vec![8000, 16000, 44100, 48000],
            supported_channels: vec![1, 2],
        };

        Ok(Arc::new(CpalAudioDevice::new(device_info, device)))
    }
}

/// Create a CPAL platform instance (only available with audio-cpal feature)
#[cfg(feature = "audio-cpal")]
pub fn create_cpal_platform() -> CpalPlatform {
    CpalPlatform::new()
}

/// Feature-gated function to check if CPAL is available
#[cfg(feature = "audio-cpal")]
pub fn is_cpal_available() -> bool {
    true
}

#[cfg(not(feature = "audio-cpal"))]
pub fn is_cpal_available() -> bool {
    false
} 