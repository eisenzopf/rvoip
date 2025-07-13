use anyhow::Result;
use clap::Parser;
use rvoip::client_core::{
    ClientConfig, ClientEventHandler, ClientError, 
    IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventInfo,
    CallAction, CallId, CallState, MediaConfig,
    ClientManager,
    AudioDeviceManager, AudioDirection, AudioFormat, AudioDevice,
    AudioStreamConfig,
    audio::device::AudioFrame as DeviceAudioFrame,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

/// Command line arguments for Audio Peer
#[derive(Parser)]
#[command(name = "audio_peer")]
#[command(about = "VoIP Audio Peer - Make and receive calls with real-time audio streaming")]
struct Args {
    /// Local IP address to bind to (e.g., 0.0.0.0 for all interfaces, 127.0.0.1 for localhost)
    #[arg(long, default_value = "127.0.0.1")]
    local_ip: String,
    
    /// Local SIP port to bind to
    #[arg(long, default_value = "5060")]
    local_port: u16,
    
    /// Local RTP port range start
    #[arg(long, default_value = "20000")]
    rtp_port_start: u16,
    
    /// Your display name (shown to the other peer)
    #[arg(long, default_value = "Peer")]
    display_name: String,
    
    /// Auto-answer delay in seconds (0 for immediate)
    #[arg(long, default_value = "1")]
    answer_delay: u64,
    
    /// Call a remote peer (provide their IP address)
    #[arg(long)]
    call: Option<String>,
    
    /// Remote peer's SIP port (when calling)
    #[arg(long, default_value = "5060")]
    remote_port: u16,
    
    /// Call duration in seconds (when calling)
    #[arg(long, default_value = "30")]
    duration: u64,
    
    /// Force full duplex mode (both mic and speakers) even in local mode
    #[arg(long)]
    force_full_duplex: bool,
    
    /// Local demo mode - listener uses speakers only, caller uses microphone only
    #[arg(long)]
    local_demo_mode: bool,
}

/// Audio mode for this peer
#[derive(Debug, Clone, Copy)]
enum AudioMode {
    /// Use microphone only (local mode sender)
    MicrophoneOnly,
    /// Use speakers only (local mode receiver)  
    SpeakersOnly,
    /// Use both microphone and speakers (remote mode or forced)
    FullDuplex,
}

/// Audio Peer - Can both make and receive calls with real audio streaming
#[derive(Clone)]
struct AudioPeerHandler {
    client_manager: Arc<RwLock<Option<Arc<ClientManager>>>>,
    audio_device_manager: Arc<RwLock<Option<Arc<AudioDeviceManager>>>>,
    call_completed: Arc<Mutex<bool>>,
    call_id: Arc<Mutex<Option<CallId>>>,
    microphone_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    speaker_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    audio_streaming_started: Arc<Mutex<bool>>,
    audio_mode: AudioMode,
    display_name: String,
    answer_delay: u64,
}

impl AudioPeerHandler {
    pub fn new(display_name: String, answer_delay: u64, audio_mode: AudioMode) -> Self {
        Self {
            client_manager: Arc::new(RwLock::new(None)),
            audio_device_manager: Arc::new(RwLock::new(None)),
            call_completed: Arc::new(Mutex::new(false)),
            call_id: Arc::new(Mutex::new(None)),
            microphone_task: Arc::new(Mutex::new(None)),
            speaker_task: Arc::new(Mutex::new(None)),
            audio_streaming_started: Arc::new(Mutex::new(false)),
            audio_mode,
            display_name,
            answer_delay,
        }
    }
    
    async fn set_client_manager(&self, client: Arc<ClientManager>) {
        *self.client_manager.write().await = Some(client);
    }
    
    async fn set_audio_device_manager(&self, manager: Arc<AudioDeviceManager>) {
        *self.audio_device_manager.write().await = Some(manager);
    }
    
    pub async fn is_call_completed(&self) -> bool {
        *self.call_completed.lock().await
    }
    
    pub async fn get_call_id(&self) -> Option<CallId> {
        self.call_id.lock().await.clone()
    }
    
    /// Start microphone capture and RTP transmission
    async fn start_microphone_capture(&self, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
        let client = self.client_manager.read().await.as_ref().ok_or_else(|| anyhow::anyhow!("Client manager not initialized"))?.clone();

        // Create audio device manager (proper lifecycle management)
        let manager = AudioDeviceManager::new().await?;
        
        // Get default microphone device
        let microphone = manager.get_default_device(AudioDirection::Input).await?;
        
        info!("🎤 [{}] Using microphone: {} ({})", 
              self.display_name, microphone.info().name, microphone.info().id);

        // Use high-quality format for local capture (48kHz mono for best quality)
        let mic_format = find_optimal_input_format(&microphone).await?;
        info!("🔧 [{}] Microphone format: {}Hz, {} channels", 
              self.display_name, mic_format.sample_rate, mic_format.channels);
        
        // Start capture using the device directly (managed by AudioDeviceManager)
        let mut audio_receiver = microphone.start_capture(mic_format).await?;
        
        // Clone references for the task
        let client_clone = client.clone();
        let call_id_clone = call_id.clone();
        let display_name = self.display_name.clone();
        
        // Start microphone capture task
        let mic_task = tokio::spawn(async move {
            info!("🎤 [{}] Starting microphone capture loop...", display_name);
            
            // Give CPAL stream time to initialize properly
            tokio::time::sleep(Duration::from_millis(200)).await;
            let mut frames_sent = 0u64;
            
            // Frame accumulator for proper 20ms frames at 8kHz (160 samples)
            let mut accumulator: Vec<i16> = Vec::with_capacity(512);
            const TARGET_FRAME_SIZE: usize = 160; // 20ms at 8kHz
            
            while let Some(device_frame) = audio_receiver.recv().await {
                // Downsample from high-quality capture (e.g., 48kHz) to 8kHz for RTP transmission
                let resampled_frame = resample_audio_frame(device_frame, 8000);
                
                // Add samples to accumulator
                accumulator.extend_from_slice(&resampled_frame.samples);
                
                // Process complete frames
                while accumulator.len() >= TARGET_FRAME_SIZE {
                    // Extract a complete 20ms frame
                    let frame_samples: Vec<i16> = accumulator.drain(..TARGET_FRAME_SIZE).collect();
                    
                    // Convert to session-core format with proper timestamp
                    let timestamp_ms = (frames_sent * 20) as u64; // Each frame is 20ms
                    let session_frame = rvoip::session_core::api::types::AudioFrame {
                        samples: frame_samples,
                        sample_rate: 8000,
                        channels: 1,
                        timestamp: timestamp_ms as u32,
                    };
                    
                    // Send frame to session-core for RTP transmission
                    if let Err(e) = client_clone.send_audio_frame(&call_id_clone, session_frame).await {
                        error!("❌ [{}] Failed to send audio frame: {}", display_name, e);
                        break;
                    }
                    
                    frames_sent += 1;
                    
                    // Log progress every 50 frames (about 1 second at 20ms frames)
                    if frames_sent % 50 == 0 {
                        info!("📡 [{}] Sent {} audio frames", display_name, frames_sent);
                    }
                }
            }
            
            info!("🎤 [{}] Microphone capture ended, total frames sent: {}", display_name, frames_sent);
            
            // Clean shutdown
            if let Err(e) = microphone.stop_capture().await {
                error!("❌ [{}] Failed to stop microphone: {}", display_name, e);
            }
        });
        
        // Store task handle for cleanup
        *self.microphone_task.lock().await = Some(mic_task);
        
        Ok(())
    }
    
    /// Start speaker playback from RTP reception
    async fn start_speaker_playback(&self, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
        let client = self.client_manager.read().await.as_ref().ok_or_else(|| anyhow::anyhow!("Client manager not initialized"))?.clone();

        // Create audio device manager (proper lifecycle management)
        let manager = AudioDeviceManager::new().await?;
        
        // Get default speaker device
        let speaker = manager.get_default_device(AudioDirection::Output).await?;
        
        info!("🔊 [{}] Using speaker: {} ({})", 
              self.display_name, speaker.info().name, speaker.info().id);

        // Use high-quality format for local playback (48kHz stereo for best quality)
        let speaker_format = find_optimal_output_format(&speaker).await?;
        info!("🔧 [{}] Speaker format: {}Hz, {} channels", 
              self.display_name, speaker_format.sample_rate, speaker_format.channels);
        
        // Start playback using the device directly (managed by AudioDeviceManager)
        let audio_sender = speaker.start_playback(speaker_format.clone()).await?;
        
        // Clone references for the task
        let display_name = self.display_name.clone();
        
        // Subscribe to incoming audio frames from RTP (do this OUTSIDE the task)
        let audio_subscriber = client.subscribe_to_audio_frames(&call_id).await?;
        
        // Start speaker playback task
        let speaker_task = tokio::spawn(async move {
            info!("🔊 [{}] Starting speaker playback loop...", display_name);
            
            let mut frames_played = 0u64;
            
            while let Ok(session_frame) = audio_subscriber.recv() {
                // Convert from session-core format to client-core format (8kHz mono from RTP)
                let mut device_frame = DeviceAudioFrame::new(
                    session_frame.samples.clone(),
                    AudioFormat::new(session_frame.sample_rate, session_frame.channels as u16, 16, 20),
                    session_frame.timestamp as u64,
                );
                
                // Upsample from 8kHz to speaker's native sample rate for high quality
                device_frame = resample_audio_frame(device_frame, speaker_format.sample_rate);
                
                // Convert from mono RTP audio to speaker's channel format (stereo)
                device_frame = convert_audio_channels(device_frame, speaker_format.channels);
                
                // Send frame to speaker
                if let Err(e) = audio_sender.send(device_frame).await {
                    error!("❌ [{}] Failed to send audio frame to speaker: {}", display_name, e);
                    break;
                }
                
                frames_played += 1;
                
                // Log progress every 50 frames (about 1 second at 20ms frames)
                if frames_played % 50 == 0 {
                    info!("🔊 [{}] Played {} audio frames", display_name, frames_played);
                }
            }
            
            info!("🔊 [{}] Speaker playback ended, total frames played: {}", display_name, frames_played);
            
            // Clean shutdown
            if let Err(e) = speaker.stop_playback().await {
                error!("❌ [{}] Failed to stop speaker: {}", display_name, e);
            }
        });
        
        // Store task handle for cleanup
        *self.speaker_task.lock().await = Some(speaker_task);
        
        Ok(())
    }
    
    /// Stop audio streaming tasks
    async fn stop_audio_streaming(&self) {
        // Stop microphone capture
        if let Some(mic_task) = self.microphone_task.lock().await.take() {
            mic_task.abort();
            info!("🎤 [{}] Microphone capture stopped", self.display_name);
        }
        
        // Stop speaker playback
        if let Some(speaker_task) = self.speaker_task.lock().await.take() {
            speaker_task.abort();
            info!("🔊 [{}] Speaker playback stopped", self.display_name);
        }
    }
}

#[async_trait::async_trait]
impl ClientEventHandler for AudioPeerHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 [{}] Incoming call: {} from {} to {}", 
            self.display_name, call_info.call_id, call_info.caller_uri, call_info.callee_uri);
        
        // Store the call ID
        *self.call_id.lock().await = Some(call_info.call_id.clone());
        
        // Auto-answer after a configurable delay
        let client_ref = Arc::clone(&self.client_manager);
        let call_id = call_info.call_id.clone();
        let answer_delay = self.answer_delay;
        let display_name = self.display_name.clone();
        
        tokio::spawn(async move {
            if answer_delay > 0 {
                tokio::time::sleep(Duration::from_secs(answer_delay)).await;
            }
            if let Some(client) = client_ref.read().await.as_ref() {
                info!("📞 [{}] Auto-answering call: {}", display_name, call_id);
                match client.answer_call(&call_id).await {
                    Ok(_) => info!("✅ [{}] Call answered successfully", display_name),
                    Err(e) => error!("❌ [{}] Failed to answer call: {}", display_name, e),
                }
            }
        });
        
        CallAction::Ignore // We'll handle it asynchronously
    }

    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        let state_emoji = match status_info.new_state {
            CallState::Initiating => "🔄",
            CallState::Ringing => "🔔",
            CallState::Connected => "✅",
            CallState::Failed => "❌",
            CallState::Cancelled => "🚫",
            CallState::Terminated => "📴",
            _ => "❓",
        };
        
        info!("{} [{}] Call {} state: {:?} → {:?}", 
            state_emoji, self.display_name, status_info.call_id, status_info.previous_state, status_info.new_state);
        
        if status_info.new_state == CallState::Connected {
            // Prevent duplicate audio streaming setup
            let mut streaming_started = self.audio_streaming_started.lock().await;
            if *streaming_started {
                info!("🚫 [{}] Audio streaming already started, ignoring duplicate event", self.display_name);
                return;
            }
            *streaming_started = true;
            
            info!("🎉 [{}] Call connected! Setting up audio streaming...", self.display_name);
            
            // Start audio transmission
            if let Some(client) = self.client_manager.read().await.as_ref() {
                // Configure audio stream
                let config = AudioStreamConfig {
                    sample_rate: 8000,
                    channels: 1,
                    codec: "PCMU".to_string(),
                    frame_size_ms: 20,
                    enable_aec: true,
                    enable_agc: true,
                    enable_vad: true,
                };
                
                match client.set_audio_stream_config(&status_info.call_id, config).await {
                    Ok(_) => info!("🔧 [{}] Audio stream configured", self.display_name),
                    Err(e) => error!("❌ [{}] Failed to configure audio stream: {}", self.display_name, e),
                }
                
                // Start the audio streaming pipeline
                match client.start_audio_stream(&status_info.call_id).await {
                    Ok(_) => info!("🎵 [{}] Audio stream started - frame-based API ready", self.display_name),
                    Err(e) => error!("❌ [{}] Failed to start audio stream: {}", self.display_name, e),
                }
                
                // NOTE: We're using the streaming API (send_audio_frame) NOT the integrated API
                info!("🎵 [{}] Audio stream ready - using frame-based streaming API", self.display_name);
                
                // Start audio based on mode (avoid hardware conflicts in local mode)
                let handler_clone = self.clone();
                let call_id_clone = status_info.call_id.clone();
                tokio::spawn(async move {
                    // Wait for media session to fully initialize before starting audio devices
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    
                    match handler_clone.audio_mode {
                        AudioMode::MicrophoneOnly => {
                            info!("🎤 [{}] Local mode: Microphone only (sender)", handler_clone.display_name);
                            if let Err(e) = handler_clone.start_microphone_capture(call_id_clone).await {
                                error!("❌ [{}] Failed to start microphone: {}", handler_clone.display_name, e);
                            }
                        }
                        AudioMode::SpeakersOnly => {
                            info!("🔊 [{}] Local mode: Speakers only (receiver)", handler_clone.display_name);
                            if let Err(e) = handler_clone.start_speaker_playback(call_id_clone).await {
                                error!("❌ [{}] Failed to start speaker: {}", handler_clone.display_name, e);
                            }
                        }
                        AudioMode::FullDuplex => {
                            info!("🎵 [{}] Remote mode: Full duplex (microphone + speakers)", handler_clone.display_name);
                            if let Err(e) = handler_clone.start_microphone_capture(call_id_clone.clone()).await {
                                error!("❌ [{}] Failed to start microphone: {}", handler_clone.display_name, e);
                            }
                            if let Err(e) = handler_clone.start_speaker_playback(call_id_clone).await {
                                error!("❌ [{}] Failed to start speaker: {}", handler_clone.display_name, e);
                            }
                        }
                    }
                });
            }
        } else if status_info.new_state == CallState::Terminated {
            info!("📴 [{}] Call terminated - stopping audio streaming", self.display_name);
            self.stop_audio_streaming().await;
            *self.call_completed.lock().await = true;
            *self.audio_streaming_started.lock().await = false;  // Reset for next call
        }
    }

    async fn on_media_event(&self, event: MediaEventInfo) {
        info!("🎵 [{}] Media event for {}: {:?}", self.display_name, event.call_id, event.event_type);
    }

    async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {
        // Not needed for peer-to-peer
    }

    async fn on_client_error(&self, error: ClientError, call_id: Option<CallId>) {
        error!("❌ [{}] Error on call {:?}: {}", self.display_name, call_id, error);
    }

    async fn on_network_event(&self, connected: bool, reason: Option<String>) {
        let status = if connected { "🌐 Connected" } else { "🔌 Disconnected" };
        info!("{} [{}] Network status changed", status, self.display_name);
        if let Some(reason) = reason {
            info!("💬 [{}] Reason: {}", self.display_name, reason);
        }
    }
}

/// Resample audio frame to target sample rate
fn resample_audio_frame(frame: DeviceAudioFrame, target_sample_rate: u32) -> DeviceAudioFrame {
    if frame.format.sample_rate == target_sample_rate {
        return frame; // No resampling needed
    }
    
    let ratio = frame.format.sample_rate as f64 / target_sample_rate as f64;
    
    if ratio > 1.0 {
        // Downsampling (e.g., 48000Hz -> 8000Hz)
        let step = ratio;
        let mut resampled_samples = Vec::new();
        
        let mut pos = 0.0;
        while (pos as usize) < frame.samples.len() {
            resampled_samples.push(frame.samples[pos as usize]);
            pos += step;
        }
        
        let mut resampled_format = frame.format.clone();
        resampled_format.sample_rate = target_sample_rate;
        
        DeviceAudioFrame::new(resampled_samples, resampled_format, frame.timestamp_ms)
    } else {
        // Upsampling (e.g., 8000Hz -> 48000Hz) 
        let repeat_count = (target_sample_rate / frame.format.sample_rate) as usize;
        let mut resampled_samples = Vec::with_capacity(frame.samples.len() * repeat_count);
        
        for sample in &frame.samples {
            for _ in 0..repeat_count {
                resampled_samples.push(*sample);
            }
        }
        
        let mut resampled_format = frame.format.clone();
        resampled_format.sample_rate = target_sample_rate;
        
        DeviceAudioFrame::new(resampled_samples, resampled_format, frame.timestamp_ms)
    }
}

/// Convert audio frame between different channel counts (from loopback example)
fn convert_audio_channels(frame: DeviceAudioFrame, target_channels: u16) -> DeviceAudioFrame {
    if frame.format.channels == target_channels {
        return frame; // No conversion needed
    }
    
    let converted_samples = if frame.format.channels == 1 && target_channels == 2 {
        // Mono to Stereo: duplicate each sample to both channels
        let mut stereo_samples = Vec::with_capacity(frame.samples.len() * 2);
        for sample in &frame.samples {
            stereo_samples.push(*sample); // Left channel
            stereo_samples.push(*sample); // Right channel
        }
        stereo_samples
    } else if frame.format.channels == 2 && target_channels == 1 {
        // Stereo to Mono: average left and right channels
        let mut mono_samples = Vec::with_capacity(frame.samples.len() / 2);
        for chunk in frame.samples.chunks_exact(2) {
            let left = chunk[0] as i32;
            let right = chunk[1] as i32;
            let mono = ((left + right) / 2) as i16;
            mono_samples.push(mono);
        }
        mono_samples
    } else {
        // Unsupported conversion, just return original
        frame.samples
    };
    
    let mut converted_format = frame.format.clone();
    converted_format.channels = target_channels;
    
    DeviceAudioFrame::new(converted_samples, converted_format, frame.timestamp_ms)
}

/// Find optimal audio format for input device (microphone)
/// Prefers mono at highest supported sample rate for best quality
async fn find_optimal_input_format(device: &Arc<dyn AudioDevice>) -> Result<AudioFormat> {
    let info = device.info();
    
    // Prefer mono for microphones
    let channels = if info.supported_channels.contains(&1) {
        1
    } else {
        *info.supported_channels.first()
            .ok_or_else(|| anyhow::anyhow!("Input device has no supported channels"))?
    };
    
    // Use highest supported sample rate for best quality
    let sample_rate = *info.supported_sample_rates.iter().max()
        .ok_or_else(|| anyhow::anyhow!("Input device has no supported sample rates"))?;
    
    Ok(AudioFormat::new(sample_rate, channels, 16, 20))
}

/// Find optimal audio format for output device (speakers)
/// Prefers stereo at highest supported sample rate for best quality
async fn find_optimal_output_format(device: &Arc<dyn AudioDevice>) -> Result<AudioFormat> {
    let info = device.info();
    
    // Prefer stereo for speakers, fall back to mono
    let channels = if info.supported_channels.contains(&2) {
        2
    } else if info.supported_channels.contains(&1) {
        1
    } else {
        *info.supported_channels.first()
            .ok_or_else(|| anyhow::anyhow!("Output device has no supported channels"))?
    };
    
    // Use highest supported sample rate for best quality
    let sample_rate = *info.supported_sample_rates.iter().max()
        .ok_or_else(|| anyhow::anyhow!("Output device has no supported sample rates"))?;
    
    Ok(AudioFormat::new(sample_rate, channels, 16, 20))
}

/// Determine audio mode based on network configuration
fn determine_audio_mode(local_ip: &str, remote_ip: Option<&str>, is_caller: bool, force_full_duplex: bool, local_demo_mode: bool) -> AudioMode {
    // If forced full duplex, always use both mic and speakers
    if force_full_duplex {
        return AudioMode::FullDuplex;
    }
    
    // If local demo mode, enforce the proper split regardless of IPs
    if local_demo_mode {
        return if is_caller {
            AudioMode::MicrophoneOnly  // Caller sends audio
        } else {
            AudioMode::SpeakersOnly    // Listener receives audio
        };
    }
    
    // If no remote IP (listener mode), we don't know yet - default to full duplex
    let Some(remote_ip) = remote_ip else {
        return AudioMode::FullDuplex;
    };
    
    // Check if this is local mode (same computer)
    let is_local_mode = remote_ip == "127.0.0.1" 
        || remote_ip == "localhost" 
        || remote_ip == local_ip
        || (local_ip == "127.0.0.1" && remote_ip == "127.0.0.1");
    
    if is_local_mode {
        // Local mode: avoid hardware conflicts
        if is_caller {
            AudioMode::MicrophoneOnly  // Caller sends audio
        } else {
            AudioMode::SpeakersOnly    // Listener receives audio
        }
    } else {
        // Remote mode: full bidirectional communication
        AudioMode::FullDuplex
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args = Args::parse();
    
    // Create logs directory
    std::fs::create_dir_all("logs")?;
    
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("audio_peer=info".parse()?)
                .add_directive("rvoip=debug".parse()?)
                .add_directive("rvoip_session_core=debug".parse()?)
                .add_directive("rvoip_media_core=debug".parse()?)
                .add_directive("rvoip_rtp_core=debug".parse()?)
        )
        .init();

    // Determine mode
    let mode = if args.call.is_some() { "CALLER" } else { "LISTENER" };
    
    info!("🚀 [{}] Starting VoIP Audio Peer in {} mode", args.display_name, mode);
    info!("🏠 [{}] Local: {}:{}, RTP: {}", args.display_name, args.local_ip, args.local_port, args.rtp_port_start);
    
    if let Some(ref remote_ip) = args.call {
        info!("📞 [{}] Will call: {}:{} for {} seconds", args.display_name, remote_ip, args.remote_port, args.duration);
    } else {
        info!("👂 [{}] Listening for incoming calls (auto-answer: {}s)", args.display_name, args.answer_delay);
    }
    
    info!("🎵 [{}] Real-time audio streaming enabled", args.display_name);

    // List available audio devices
    let audio_manager = AudioDeviceManager::new().await?;
    
    let input_devices = audio_manager.list_devices(AudioDirection::Input).await?;
    let output_devices = audio_manager.list_devices(AudioDirection::Output).await?;
    
    info!("🔍 [{}] Found {} input device(s) and {} output device(s)", 
         args.display_name, input_devices.len(), output_devices.len());
    
    if input_devices.is_empty() || output_devices.is_empty() {
        error!("❌ [{}] No audio devices found! Please ensure microphone and speakers are connected.", args.display_name);
        return Err(anyhow::anyhow!("No audio devices available"));
    }

    // Create configuration using command line arguments
    let local_sip_addr = format!("{}:{}", args.local_ip, args.local_port);
    let local_media_addr = format!("{}:{}", args.local_ip, args.rtp_port_start);
    
    let config = ClientConfig::new()
        .with_sip_addr(local_sip_addr.parse()?)
        .with_media_addr(local_media_addr.parse()?)
        .with_user_agent(format!("RVOIP-{}/1.0", args.display_name))
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            dtmf_enabled: true,
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            rtp_port_start: args.rtp_port_start,
            rtp_port_end: args.rtp_port_start + 100,
            ..Default::default()
        });

    // Determine audio mode based on network configuration
    let is_caller = args.call.is_some();
    let audio_mode = determine_audio_mode(&args.local_ip, args.call.as_deref(), is_caller, args.force_full_duplex, args.local_demo_mode);
    
    // Log the audio mode
    match audio_mode {
        AudioMode::MicrophoneOnly => info!("🎤 [{}] Audio Mode: Microphone only (local sender)", args.display_name),
        AudioMode::SpeakersOnly => info!("🔊 [{}] Audio Mode: Speakers only (local receiver)", args.display_name),
        AudioMode::FullDuplex => info!("🎵 [{}] Audio Mode: Full duplex (remote or forced)", args.display_name),
    }

    // Create handler and client
    let handler = Arc::new(AudioPeerHandler::new(args.display_name.clone(), args.answer_delay, audio_mode));
    let client = ClientManager::new(config).await?;
    
    handler.set_client_manager(client.clone()).await;
    handler.set_audio_device_manager(Arc::new(audio_manager)).await;
    client.set_event_handler(handler.clone()).await;
    
    // Start the client
    client.start().await?;
    info!("✅ [{}] Client started and ready", args.display_name);

    // Handle calling vs listening mode
    if let Some(remote_ip) = args.call {
        // CALLER MODE: Make a call to the remote peer
        info!("⏳ [{}] Waiting 3 seconds for remote peer to be ready...", args.display_name);
        tokio::time::sleep(Duration::from_secs(3)).await;

        info!("📞 [{}] Initiating call to {}:{}...", args.display_name, remote_ip, args.remote_port);
        let from_uri = format!("sip:{}@{}:{}", args.display_name.to_lowercase(), args.local_ip, args.local_port);
        let to_uri = format!("sip:peer@{}:{}", remote_ip, args.remote_port);
        
        let call_id = client.make_call(from_uri, to_uri, None).await?;
        info!("📞 [{}] Call initiated with ID: {}", args.display_name, call_id);

        // Let the call run for specified duration
        info!("⏰ [{}] Call will run for {} seconds with real audio streaming...", args.display_name, args.duration);
        info!("🎤 [{}] Speak into your microphone - your voice will be transmitted!", args.display_name);
        info!("🔊 [{}] You should hear audio from the remote peer through your speakers!", args.display_name);
        
        tokio::time::sleep(Duration::from_secs(args.duration)).await;

        // Get final statistics
        if let Ok(Some(rtp_stats)) = client.get_rtp_statistics(&call_id).await {
            info!("📊 [{}] Final RTP Stats - Sent: {} packets ({} bytes), Received: {} packets ({} bytes)",
                args.display_name, rtp_stats.packets_sent, rtp_stats.bytes_sent, 
                rtp_stats.packets_received, rtp_stats.bytes_received);
        }

        // Terminate the call
        info!("📴 [{}] Terminating call...", args.display_name);
        client.hangup_call(&call_id).await?;

        // Wait for call termination
        let mut attempts = 0;
        while !handler.is_call_completed().await && attempts < 10 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            attempts += 1;
        }

        info!("✅ [{}] Call completed successfully!", args.display_name);
    } else {
        // LISTENER MODE: Wait for incoming calls
        info!("⏳ [{}] Waiting for incoming calls...", args.display_name);
        info!("🎤 [{}] When connected, speak into your microphone!", args.display_name);
        info!("🔊 [{}] You should hear audio from the caller through your speakers!", args.display_name);
        
        let mut timeout_counter = 0;
        
        while !handler.is_call_completed().await && timeout_counter < 300 {  // 5 minutes timeout
            tokio::time::sleep(Duration::from_secs(1)).await;
            timeout_counter += 1;
            
            // Log periodic status updates
            if timeout_counter % 30 == 0 {
                info!("⏰ [{}] Still listening... ({} seconds elapsed)", args.display_name, timeout_counter);
            }
        }

        // Get final statistics if we had a call
        if let Some(call_id) = handler.get_call_id().await {
            if let Ok(Some(rtp_stats)) = client.get_rtp_statistics(&call_id).await {
                info!("📊 [{}] Final RTP Stats - Sent: {} packets ({} bytes), Received: {} packets ({} bytes)",
                    args.display_name, rtp_stats.packets_sent, rtp_stats.bytes_sent, 
                    rtp_stats.packets_received, rtp_stats.bytes_received);
            }
        }

        if handler.is_call_completed().await {
            info!("✅ [{}] Call completed successfully!", args.display_name);
        } else {
            warn!("⚠️ [{}] Timed out - no call received", args.display_name);
        }
    }

    // Stop the client
    client.stop().await?;
    info!("🎉 [{}] VoIP Audio Peer shutdown complete!", args.display_name);

    Ok(())
} 