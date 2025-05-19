use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

use crate::codec::audio::common::AudioFormat;
use crate::engine::audio::device::{AudioDevice, AudioDeviceManager};
use crate::error::Result;
use crate::processing::pipeline::{AudioPipeline, PipelineConfig};

/// Audio playback events
#[derive(Debug, Clone)]
pub enum AudioPlaybackEvent {
    /// Buffer underrun occurred
    Underrun,
    /// Buffer level changed
    BufferLevel(f32),
    /// Playback started
    Started,
    /// Playback stopped
    Stopped,
    /// Error occurred
    Error(String),
}

/// Audio playback configuration
#[derive(Debug, Clone)]
pub struct AudioPlaybackConfig {
    /// Audio device ID (None for default)
    pub device_id: Option<String>,
    /// Audio format
    pub format: AudioFormat,
    /// Packet loss concealment
    pub packet_loss_concealment: bool,
    /// Target buffer size in milliseconds
    pub buffer_size_ms: u32,
    /// Minimum buffer size in milliseconds
    pub min_buffer_size_ms: u32,
    /// Maximum buffer size in milliseconds
    pub max_buffer_size_ms: u32,
    /// Playback interval in milliseconds
    pub interval_ms: u32,
}

impl Default for AudioPlaybackConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            format: AudioFormat::pcm_telephony(),
            packet_loss_concealment: true,
            buffer_size_ms: 60,
            min_buffer_size_ms: 20,
            max_buffer_size_ms: 200,
            interval_ms: 20,
        }
    }
}

/// Audio playback handle
pub struct AudioPlayback {
    /// Playback configuration
    config: AudioPlaybackConfig,
    /// Audio device
    device: Option<AudioDevice>,
    /// Playback thread handle
    playback_thread: Option<thread::JoinHandle<()>>,
    /// Data queue for playback
    data_queue: Arc<Mutex<VecDeque<Bytes>>>,
    /// Event sender
    event_sender: mpsc::UnboundedSender<AudioPlaybackEvent>,
    /// Event receiver
    event_receiver: Option<mpsc::UnboundedReceiver<AudioPlaybackEvent>>,
    /// Audio processing pipeline
    pipeline: Option<AudioPipeline>,
    /// Running state
    running: Arc<Mutex<bool>>,
    /// Current buffer level in milliseconds
    buffer_level_ms: Arc<Mutex<u32>>,
}

impl AudioPlayback {
    /// Create a new audio playback
    pub fn new(config: AudioPlaybackConfig) -> Result<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        
        Ok(Self {
            config,
            device: None,
            playback_thread: None,
            data_queue: Arc::new(Mutex::new(VecDeque::new())),
            event_sender: tx,
            event_receiver: Some(rx),
            pipeline: None,
            running: Arc::new(Mutex::new(false)),
            buffer_level_ms: Arc::new(Mutex::new(0)),
        })
    }
    
    /// Start playback
    pub fn start(&mut self) -> Result<mpsc::UnboundedReceiver<AudioPlaybackEvent>> {
        // Check if already running
        if *self.running.lock().unwrap() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Audio playback already running"
            ).into());
        }
        
        // Open the audio device
        let device_manager = AudioDeviceManager::instance();
        let device_manager = device_manager.lock().unwrap();
        
        let device = device_manager.open_output(
            self.config.device_id.as_deref(),
            self.config.format,
            calculate_buffer_size(
                self.config.format,
                self.config.buffer_size_ms,
            )
        )?;
        
        // Create audio processing pipeline if needed
        if self.config.packet_loss_concealment {
            let pipeline_config = PipelineConfig {
                input_format: self.config.format,
                output_format: self.config.format,
                use_vad: false,
                noise_reduction: false,
                auto_gain: false,
                echo_cancellation: false,
                packet_loss_concealment: true,
            };
            
            self.pipeline = Some(AudioPipeline::new(pipeline_config));
        }
        
        // Mark as running
        *self.running.lock().unwrap() = true;
        
        // Start the device
        let mut device = device;
        device.start()?;
        
        // Store device
        self.device = Some(device);
        
        // Start the playback thread
        let running = self.running.clone();
        let event_sender = self.event_sender.clone();
        let data_queue = self.data_queue.clone();
        let interval_ms = self.config.interval_ms;
        let device_id = self.config.device_id.clone();
        let format = self.config.format;
        let buffer_level_ms = self.buffer_level_ms.clone();
        let min_buffer_ms = self.config.min_buffer_size_ms;
        let pipeline = self.pipeline.take();
        
        let frame_size = calculate_buffer_size(format, interval_ms);
        let bytes_per_frame = frame_size * format.bytes_per_sample();
        
        // Send started event
        let _ = event_sender.send(AudioPlaybackEvent::Started);
        
        let thread = thread::spawn(move || {
            let mut empty_buffer = vec![127u8; bytes_per_frame]; // Silence
            let mut pipeline = pipeline;
            
            debug!("Audio playback thread started for device: {:?}", device_id);
            
            while *running.lock().unwrap() {
                let playback_start = Instant::now();
                
                // Get audio data from queue
                let mut data_to_play = None;
                let mut underrun = false;
                
                {
                    let mut queue = data_queue.lock().unwrap();
                    if let Some(data) = queue.pop_front() {
                        data_to_play = Some(data);
                        
                        // Update buffer level
                        let mut buf_level = buffer_level_ms.lock().unwrap();
                        *buf_level = queue.len() as u32 * interval_ms;
                        
                        // Send buffer level event
                        let target_ms = self.config.buffer_size_ms;
                        let level_pct = (*buf_level as f32 / target_ms as f32).clamp(0.0, 1.0);
                        let _ = event_sender.send(AudioPlaybackEvent::BufferLevel(level_pct));
                    } else {
                        // Buffer underrun
                        underrun = true;
                        
                        // Reset buffer level
                        let mut buf_level = buffer_level_ms.lock().unwrap();
                        *buf_level = 0;
                        
                        // Send buffer level event
                        let _ = event_sender.send(AudioPlaybackEvent::BufferLevel(0.0));
                    }
                }
                
                if underrun {
                    // Send underrun event
                    let _ = event_sender.send(AudioPlaybackEvent::Underrun);
                    
                    // Use silence for playback
                    data_to_play = Some(Bytes::from(empty_buffer.clone()));
                }
                
                // Process the audio if needed
                let final_data = if let Some(mut processor) = pipeline.as_mut() {
                    if let Some(data) = data_to_play {
                        match processor.process(&data) {
                            Ok(processed) => processed,
                            Err(e) => {
                                error!("Audio processing failed: {}", e);
                                if let Err(e) = event_sender.send(AudioPlaybackEvent::Error(e.to_string())) {
                                    error!("Failed to send error event: {}", e);
                                }
                                Bytes::from(empty_buffer.clone())
                            }
                        }
                    } else {
                        Bytes::from(empty_buffer.clone())
                    }
                } else {
                    data_to_play.unwrap_or_else(|| Bytes::from(empty_buffer.clone()))
                };
                
                // In a real implementation, this would write to the device
                // For this stub, we just simulate playback
                
                // Sleep for the remaining interval time
                let elapsed = playback_start.elapsed();
                let target_interval = Duration::from_millis(interval_ms as u64);
                if elapsed < target_interval {
                    thread::sleep(target_interval - elapsed);
                } else {
                    // Playback is falling behind
                    warn!("Playback is falling behind: elapsed={}ms, interval={}ms",
                          elapsed.as_millis(), interval_ms);
                }
            }
            
            debug!("Audio playback thread stopped");
        });
        
        self.playback_thread = Some(thread);
        
        // Return the event receiver
        self.event_receiver.take().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "Event receiver already taken"
            ).into()
        })
    }
    
    /// Stop playback
    pub fn stop(&mut self) -> Result<()> {
        // Check if running
        if !*self.running.lock().unwrap() {
            return Ok(());
        }
        
        // Mark as not running
        *self.running.lock().unwrap() = false;
        
        // Wait for the playback thread to end
        if let Some(thread) = self.playback_thread.take() {
            thread.join().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to join playback thread"
                )
            })?;
        }
        
        // Stop and close the device
        if let Some(mut device) = self.device.take() {
            device.stop()?;
            device.close()?;
        }
        
        // Clear the queue
        let mut queue = self.data_queue.lock().unwrap();
        queue.clear();
        
        // Reset buffer level
        let mut buf_level = self.buffer_level_ms.lock().unwrap();
        *buf_level = 0;
        
        // Send stopped event
        let _ = self.event_sender.send(AudioPlaybackEvent::Stopped);
        
        info!("Audio playback stopped");
        
        Ok(())
    }
    
    /// Queue audio data for playback
    pub fn queue(&self, data: Bytes) -> Result<()> {
        // Check if running
        if !*self.running.lock().unwrap() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Audio playback not running"
            ).into());
        }
        
        // Add to queue
        let mut queue = self.data_queue.lock().unwrap();
        queue.push_back(data);
        
        // Update buffer level
        let mut buf_level = self.buffer_level_ms.lock().unwrap();
        *buf_level = queue.len() as u32 * self.config.interval_ms;
        
        Ok(())
    }
    
    /// Get the playback configuration
    pub fn config(&self) -> &AudioPlaybackConfig {
        &self.config
    }
    
    /// Check if playback is active
    pub fn is_active(&self) -> bool {
        *self.running.lock().unwrap()
    }
    
    /// Get the current buffer level in milliseconds
    pub fn buffer_level_ms(&self) -> u32 {
        *self.buffer_level_ms.lock().unwrap()
    }
    
    /// Get the current buffer level as a percentage
    pub fn buffer_level_pct(&self) -> f32 {
        let buf_ms = *self.buffer_level_ms.lock().unwrap();
        let target_ms = self.config.buffer_size_ms;
        (buf_ms as f32 / target_ms as f32).clamp(0.0, 1.0)
    }
    
    /// Check if buffer is ready for playback
    pub fn is_buffer_ready(&self) -> bool {
        let buf_ms = *self.buffer_level_ms.lock().unwrap();
        buf_ms >= self.config.min_buffer_size_ms
    }
}

impl Drop for AudioPlayback {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Calculate buffer size in frames based on format and duration
fn calculate_buffer_size(format: AudioFormat, duration_ms: u32) -> usize {
    (format.sample_rate.as_hz() as u64 * duration_ms as u64 / 1000) as usize
} 