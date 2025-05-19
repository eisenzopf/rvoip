use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace};

use crate::codec::audio::common::AudioFormat;
use crate::engine::audio::device::{AudioDevice, AudioDeviceManager};
use crate::error::Result;
use crate::processing::pipeline::{AudioPipeline, PipelineConfig, PipelineEvent};

/// Audio capture events
#[derive(Debug, Clone)]
pub enum AudioCaptureEvent {
    /// Audio data captured
    Data(Bytes),
    /// Silence detected
    Silence,
    /// Speech detected
    Speech,
    /// Level change
    LevelChange(f32),
    /// Error occurred
    Error(String),
}

/// Audio capture configuration
#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    /// Audio device ID (None for default)
    pub device_id: Option<String>,
    /// Audio format
    pub format: AudioFormat,
    /// Whether to use voice activity detection
    pub use_vad: bool,
    /// Whether to use noise reduction
    pub noise_reduction: bool,
    /// Whether to use automatic gain control
    pub auto_gain_control: bool,
    /// Buffer size in milliseconds
    pub buffer_size_ms: u32,
    /// Capture interval in milliseconds
    pub interval_ms: u32,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            format: AudioFormat::pcm_telephony(),
            use_vad: true,
            noise_reduction: true,
            auto_gain_control: true,
            buffer_size_ms: 20,
            interval_ms: 20,
        }
    }
}

/// Audio capture handle
pub struct AudioCapture {
    /// Capture configuration
    config: AudioCaptureConfig,
    /// Audio device
    device: Option<AudioDevice>,
    /// Capture thread handle
    capture_thread: Option<thread::JoinHandle<()>>,
    /// Event sender
    event_sender: mpsc::UnboundedSender<AudioCaptureEvent>,
    /// Event receiver
    event_receiver: Option<mpsc::UnboundedReceiver<AudioCaptureEvent>>,
    /// Audio processing pipeline
    pipeline: Option<AudioPipeline>,
    /// Running state
    running: Arc<Mutex<bool>>,
}

impl AudioCapture {
    /// Create a new audio capture
    pub fn new(config: AudioCaptureConfig) -> Result<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        
        Ok(Self {
            config,
            device: None,
            capture_thread: None,
            event_sender: tx,
            event_receiver: Some(rx),
            pipeline: None,
            running: Arc::new(Mutex::new(false)),
        })
    }
    
    /// Start capturing audio
    pub fn start(&mut self) -> Result<mpsc::UnboundedReceiver<AudioCaptureEvent>> {
        // Check if already running
        if *self.running.lock().unwrap() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Audio capture already running"
            ).into());
        }
        
        // Open the audio device
        let device_manager = AudioDeviceManager::instance();
        let device_manager = device_manager.lock().unwrap();
        
        let device = device_manager.open_input(
            self.config.device_id.as_deref(),
            self.config.format,
            calculate_buffer_size(
                self.config.format,
                self.config.buffer_size_ms,
            )
        )?;
        
        // Create audio processing pipeline
        let pipeline_config = PipelineConfig {
            input_format: self.config.format,
            output_format: self.config.format,
            use_vad: self.config.use_vad,
            noise_reduction: self.config.noise_reduction,
            auto_gain: self.config.auto_gain_control,
            echo_cancellation: false,
            packet_loss_concealment: false,
        };
        
        let mut pipeline = AudioPipeline::new(pipeline_config);
        
        // Set up pipeline events
        let event_sender = self.event_sender.clone();
        pipeline.set_event_callback(Arc::new(move |event| {
            match event {
                PipelineEvent::VoiceActivityChanged(state) => {
                    let event = match state {
                        crate::processing::audio::vad::VadState::Speech => AudioCaptureEvent::Speech,
                        crate::processing::audio::vad::VadState::NonSpeech => AudioCaptureEvent::Silence,
                    };
                    let _ = event_sender.send(event);
                }
                PipelineEvent::LevelChanged(level) => {
                    let _ = event_sender.send(AudioCaptureEvent::LevelChange(level));
                }
                _ => {}
            }
        }));
        
        // Store pipeline
        self.pipeline = Some(pipeline);
        
        // Mark as running
        *self.running.lock().unwrap() = true;
        
        // Start the device
        let mut device = device;
        device.start()?;
        
        // Store device
        self.device = Some(device);
        
        // Start the capture thread
        let running = self.running.clone();
        let event_sender = self.event_sender.clone();
        let interval_ms = self.config.interval_ms;
        let device_id = self.config.device_id.clone();
        let format = self.config.format;
        let buffer_size = calculate_buffer_size(format, self.config.buffer_size_ms);
        
        let pipeline = self.pipeline.take().unwrap();
        
        let thread = thread::spawn(move || {
            let mut buffer = vec![0u8; buffer_size * format.bytes_per_sample()];
            let mut pipeline = pipeline;
            
            debug!("Audio capture thread started for device: {:?}", device_id);
            
            while *running.lock().unwrap() {
                let capture_start = Instant::now();
                
                // In a real implementation, this would read from the device
                // For this stub, we generate silent audio
                for i in 0..buffer.len() {
                    buffer[i] = 128; // Silence for 8-bit unsigned PCM
                }
                
                // Process the captured audio
                match pipeline.process(&buffer) {
                    Ok(processed) => {
                        // Send the processed audio data
                        if let Err(e) = event_sender.send(AudioCaptureEvent::Data(processed)) {
                            error!("Failed to send captured audio: {}", e);
                            break;
                        }
                    },
                    Err(e) => {
                        error!("Audio processing failed: {}", e);
                        if let Err(e) = event_sender.send(AudioCaptureEvent::Error(e.to_string())) {
                            error!("Failed to send error event: {}", e);
                        }
                    }
                }
                
                // Sleep for the remaining interval time
                let elapsed = capture_start.elapsed();
                let target_interval = Duration::from_millis(interval_ms as u64);
                if elapsed < target_interval {
                    thread::sleep(target_interval - elapsed);
                }
            }
            
            debug!("Audio capture thread stopped");
        });
        
        self.capture_thread = Some(thread);
        
        // Return the event receiver
        self.event_receiver.take().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "Event receiver already taken"
            ).into()
        })
    }
    
    /// Stop capturing audio
    pub fn stop(&mut self) -> Result<()> {
        // Check if running
        if !*self.running.lock().unwrap() {
            return Ok(());
        }
        
        // Mark as not running
        *self.running.lock().unwrap() = false;
        
        // Wait for the capture thread to end
        if let Some(thread) = self.capture_thread.take() {
            thread.join().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to join capture thread"
                )
            })?;
        }
        
        // Stop and close the device
        if let Some(mut device) = self.device.take() {
            device.stop()?;
            device.close()?;
        }
        
        info!("Audio capture stopped");
        
        Ok(())
    }
    
    /// Get the capture configuration
    pub fn config(&self) -> &AudioCaptureConfig {
        &self.config
    }
    
    /// Check if capturing is active
    pub fn is_active(&self) -> bool {
        *self.running.lock().unwrap()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Calculate buffer size in frames based on format and duration
fn calculate_buffer_size(format: AudioFormat, duration_ms: u32) -> usize {
    (format.sample_rate.as_hz() as u64 * duration_ms as u64 / 1000) as usize
} 