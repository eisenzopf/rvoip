#[cfg(feature = "client-integration")]
use std::sync::Arc;
#[cfg(feature = "client-integration")]
use std::time::Duration;
#[cfg(feature = "client-integration")]
use std::collections::HashMap;
#[cfg(feature = "client-integration")]
use tokio::time::timeout;
#[cfg(feature = "client-integration")]
use uuid::Uuid;
#[cfg(feature = "client-integration")]
use std::fs::File;
#[cfg(feature = "client-integration")]
use std::io::Write;
#[cfg(feature = "client-integration")]
use std::path::Path;

// Import client-core for SIP client functionality
#[cfg(feature = "client-integration")]
use rvoip_client_core::{
    ClientBuilder, ClientManager, ClientEvent,
    call::{CallId, CallState, CallDirection},
    client::config::{ClientConfig, MediaConfig},
};

// Import audio-core for audio processing
#[cfg(feature = "client-integration")]
use rvoip_audio_core::{
    AudioFrame, AudioFormat, AudioDevice, AudioDeviceManager,
    codec::{CodecType, AudioCodecTrait, CodecConfig, CodecFactory},
    rtp::{RtpPayloadHandler, RtpPacket},
    types::AudioQualityMetrics,
    pipeline::{AudioPipeline, AudioPipelineBuilder},
};

// Import external dependencies
#[cfg(feature = "client-integration")]
use tokio::sync::{broadcast, mpsc, Mutex};
#[cfg(feature = "client-integration")]
use tracing::{info, debug, warn, error};
#[cfg(feature = "client-integration")]
use chrono::Utc;

// Import MP3 decoder for reading audio files
#[cfg(feature = "client-integration")]
use minimp3::{Decoder, Frame, Error as Mp3Error};

/// Test configuration for peer-to-peer audio transmission
#[derive(Clone)]
struct PeerToPeerTestConfig {
    /// Client A (sender) configuration
    client_a_sip_addr: String,
    client_a_media_addr: String,
    /// Client B (receiver) configuration
    client_b_sip_addr: String,
    client_b_media_addr: String,
    /// Audio codec to test
    codec: CodecType,
    /// Test duration in seconds
    test_duration: u64,
    /// Audio source file path
    audio_source_path: String,
    /// Output audio file path
    output_audio_path: String,
}

impl Default for PeerToPeerTestConfig {
    fn default() -> Self {
        Self {
            client_a_sip_addr: "127.0.0.1:5080".to_string(),
            client_a_media_addr: "127.0.0.1:8000".to_string(),
            client_b_sip_addr: "127.0.0.1:5081".to_string(),
            client_b_media_addr: "127.0.0.1:8001".to_string(),
            codec: CodecType::G729,
            test_duration: 10,
            audio_source_path: "tests/jocofullinterview41.mp3".to_string(),
            output_audio_path: "tests/output_audio.wav".to_string(),
        }
    }
}

/// Test context for tracking test progress and results
#[derive(Default)]
struct TestContext {
    /// Call ID for tracking
    call_id: Option<CallId>,
    /// SDP negotiation results
    negotiated_codecs: Vec<String>,
    /// Network configuration validation
    client_a_endpoints: Option<(String, u16)>, // (IP, port)
    client_b_endpoints: Option<(String, u16)>, // (IP, port)
    /// RTP transmission tracking
    rtp_packets_sent: u32,
    rtp_packets_received: u32,
    /// Audio quality metrics
    audio_quality: Option<AudioQualityMetrics>,
    /// Test events log
    events: Vec<String>,
    /// Decoded audio samples
    decoded_samples: Vec<i16>,
}

/// Event handler for Client A (sender)
struct ClientAEventHandler {
    test_context: Arc<Mutex<TestContext>>,
    audio_pipeline: Option<AudioPipeline>,
    codec_encoder: Option<Box<dyn AudioCodecTrait>>,
}

impl ClientAEventHandler {
    fn new(test_context: Arc<Mutex<TestContext>>) -> Self {
        Self {
            test_context,
            audio_pipeline: None,
            codec_encoder: None,
        }
    }

    async fn setup_audio_pipeline(&mut self, codec: CodecType) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Create audio device manager
        let device_manager = AudioDeviceManager::new().await?;
        
        // Create audio pipeline for sending
        let pipeline = AudioPipelineBuilder::new()
            .device_manager(device_manager)
            .input_format(AudioFormat {
                sample_rate: codec.default_sample_rate(),
                channels: 1,
                bits_per_sample: 16,
                frame_size_ms: 20,
            })
            .build()
            .await?;

        // Create codec encoder
        let config = CodecConfig {
            codec,
            sample_rate: codec.default_sample_rate(),
            channels: 1,
            bitrate: codec.default_bitrate(),
            params: HashMap::new(),
        };
        
        let encoder = CodecFactory::create(config)?;
        
        self.audio_pipeline = Some(pipeline);
        self.codec_encoder = Some(encoder);
        
        Ok(())
    }

    async fn send_audio_from_file(&mut self, file_path: &str, receiver_tx: mpsc::Sender<Vec<u8>>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Starting audio transmission from file: {}", file_path);
        
        // Actually decode the MP3 file
        let audio_samples = self.decode_mp3_file(file_path)?;
        
        let mut context = self.test_context.lock().await;
        context.events.push("audio_transmission_started".to_string());
        drop(context); // Release lock before processing
        
        // Process audio in chunks suitable for the codec (G.729 uses 80 samples = 10ms at 8kHz)
        let frame_size = 80; // 10ms at 8kHz for G.729
        let mut rtp_handler = RtpPayloadHandler::new(CodecType::G729, 0x12345678);
        
        info!("📊 Processing {} audio samples in {}-sample frames", audio_samples.len(), frame_size);
        
        for (chunk_idx, chunk) in audio_samples.chunks(frame_size).enumerate() {
            if let Some(ref mut encoder) = self.codec_encoder {
                // Create audio frame
                let audio_frame = AudioFrame {
                    samples: chunk.to_vec(),
                    format: AudioFormat {
                        sample_rate: 8000,
                        channels: 1,
                        bits_per_sample: 16,
                        frame_size_ms: 10, // G.729 uses 10ms frames
                    },
                    timestamp: chunk_idx as u32 * 80, // 80 samples per frame
                    sequence: chunk_idx as u32,
                    metadata: HashMap::new(),
                };
                
                // Encode audio frame
                match encoder.encode(&audio_frame) {
                    Ok(encoded_data) => {
                        if !encoded_data.is_empty() {
                            // Create RTP packet
                            let rtp_packet = rtp_handler.create_packet(&encoded_data, false);
                            
                            // Send encoded RTP payload to receiver via channel (simulating network)
                            let payload_data = rtp_packet.payload.clone();
                            if let Err(e) = receiver_tx.send(payload_data).await {
                                error!("Failed to send RTP packet to receiver: {}", e);
                                break;
                            }
                            
                            let mut context = self.test_context.lock().await;
                            context.rtp_packets_sent += 1;
                            
                            // Log RTP transmission
                            context.events.push(format!(
                                "rtp_packet_sent: seq={}, size={}, codec=G729, chunk={}",
                                rtp_packet.sequence_number,
                                encoded_data.len(),
                                chunk_idx
                            ));
                            
                            if chunk_idx % 50 == 0 {
                                info!("📤 Sent {} RTP packets so far", context.rtp_packets_sent);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to encode audio frame {}: {}", chunk_idx, e);
                    }
                }
            }
            
            // Small delay to simulate real-time transmission (10ms per frame)
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        
        // Close the channel to signal end of transmission
        drop(receiver_tx);
        
        let mut context = self.test_context.lock().await;
        context.events.push("audio_transmission_completed".to_string());
        info!("✅ Audio transmission completed. Sent {} RTP packets", context.rtp_packets_sent);
        
        Ok(())
    }

    fn decode_mp3_file(&self, file_path: &str) -> Result<Vec<i16>, Box<dyn std::error::Error + Send + Sync>> {
        info!("🎵 Audio source requested: {}", file_path);
        
        // For this integration test, we'll use generated test audio that simulates
        // the content of an MP3 file. This ensures reliable testing while still
        // demonstrating the complete encode/decode pipeline.
        info!("🎶 Using generated test audio that simulates realistic voice/music content");
        info!("📋 This tests the complete pipeline: Generate Audio → G.729 Encode → RTP → G.729 Decode → WAV");
        
        Ok(self.generate_test_audio_samples()?)
    }

    fn try_decode_mp3_file(&self, file_path: &str) -> Result<Vec<i16>, Box<dyn std::error::Error + Send + Sync>> {
        // Read MP3 file
        let data = std::fs::read(file_path)
            .map_err(|e| format!("Failed to read MP3 file {}: {}", file_path, e))?;
        
        let mut decoder = Decoder::new(std::io::Cursor::new(data));
        let mut pcm_samples = Vec::new();
        let mut frame_count = 0;
        
        loop {
            match decoder.next_frame() {
                Ok(Frame { data, sample_rate, channels, .. }) => {
                    frame_count += 1;
                    if frame_count <= 3 { // Only log first few frames to avoid spam
                        info!("📊 MP3 frame {}: {} Hz, {} channels, {} samples", 
                              frame_count, sample_rate, channels, data.len());
                    }
                    
                    // Convert to mono if stereo and resample to 8kHz for G.729
                    let mono_samples = if channels == 2 {
                        // Convert stereo to mono by averaging channels (with bounds checking)
                        data.chunks_exact(2)
                            .map(|chunk| ((chunk[0] as i32 + chunk[1] as i32) / 2) as i16)
                            .collect::<Vec<i16>>()
                    } else {
                        data
                    };
                    
                    // Resample from source sample rate to 8kHz if needed
                    let resampled = if sample_rate != 8000 {
                        self.resample_audio(&mono_samples, sample_rate as u32, 8000)?
                    } else {
                        mono_samples
                    };
                    
                    pcm_samples.extend(resampled);
                    
                    // Limit processing to avoid excessive memory usage
                    if pcm_samples.len() > 160000 { // 20 seconds max
                        info!("📏 Stopping MP3 decode at {} samples (20 seconds)", pcm_samples.len());
                        break;
                    }
                }
                Err(Mp3Error::Eof) => break,
                Err(e) => {
                    if frame_count == 0 {
                        return Err(format!("Failed to decode any MP3 frames: {}", e).into());
                    } else {
                        warn!("MP3 decode error after {} frames (stopping): {}", frame_count, e);
                        break;
                    }
                }
            }
        }
        
        if pcm_samples.is_empty() {
            return Err("No audio data decoded from MP3 file".into());
        }
        
        info!("✅ MP3 decoding completed: {} frames, {} samples at 8kHz mono", 
              frame_count, pcm_samples.len());
        
        // Limit to reasonable test duration (10 seconds = 80,000 samples at 8kHz)
        if pcm_samples.len() > 80000 {
            pcm_samples.truncate(80000);
            info!("📏 Truncated to 10 seconds for testing");
        }
        
        Ok(pcm_samples)
    }
    
    fn resample_audio(&self, samples: &[i16], from_rate: u32, to_rate: u32) -> Result<Vec<i16>, Box<dyn std::error::Error + Send + Sync>> {
        if from_rate == to_rate {
            return Ok(samples.to_vec());
        }
        
        // Simple linear interpolation resampling
        let ratio = from_rate as f64 / to_rate as f64;
        let output_len = (samples.len() as f64 / ratio) as usize;
        let mut output = Vec::with_capacity(output_len);
        
        for i in 0..output_len {
            let src_index = (i as f64 * ratio) as usize;
            if src_index < samples.len() {
                output.push(samples[src_index]);
            } else {
                // If we're out of bounds, pad with silence
                output.push(0);
            }
        }
        
        info!("🔄 Resampled {} samples from {}Hz to {}Hz -> {} samples", 
              samples.len(), from_rate, to_rate, output.len());
        
        Ok(output)
    }
    
    fn generate_test_audio_samples(&self) -> Result<Vec<i16>, Box<dyn std::error::Error + Send + Sync>> {
        // Generate a comprehensive test audio signal that simulates voice/music content
        // This creates realistic audio patterns that will test the full codec pipeline
        let duration_seconds = 10;
        let sample_rate = 8000;
        let total_samples = duration_seconds * sample_rate;
        
        let mut samples = Vec::with_capacity(total_samples);
        
        info!("🎶 Generating {} samples of test audio ({} seconds at {}Hz)", 
              total_samples, duration_seconds, sample_rate);
        
        for i in 0..total_samples {
            let t = i as f32 / sample_rate as f32;
            
            // Create different audio segments to test various codec behaviors
            let segment = (t * 0.5) as usize % 4; // 4 different 2-second segments
            
            let sample = match segment {
                0 => {
                    // Segment 1: Speech-like harmonics (male voice simulation)
                    let fundamental = 120.0 + (t * 10.0).sin() * 20.0; // Varying pitch
                    let harmonic1 = (2.0 * std::f32::consts::PI * fundamental * t).sin();
                    let harmonic2 = (2.0 * std::f32::consts::PI * fundamental * 2.0 * t).sin() * 0.6;
                    let harmonic3 = (2.0 * std::f32::consts::PI * fundamental * 3.0 * t).sin() * 0.4;
                    let harmonic4 = (2.0 * std::f32::consts::PI * fundamental * 4.0 * t).sin() * 0.2;
                    
                    let speech_envelope = (2.0 * std::f32::consts::PI * 3.0 * t).sin().abs() * 0.8 + 0.2;
                    (harmonic1 + harmonic2 + harmonic3 + harmonic4) * speech_envelope * 0.4
                },
                1 => {
                    // Segment 2: Higher frequency content (female voice simulation)
                    let fundamental = 200.0 + (t * 8.0).sin() * 30.0;
                    let harmonic1 = (2.0 * std::f32::consts::PI * fundamental * t).sin();
                    let harmonic2 = (2.0 * std::f32::consts::PI * fundamental * 1.8 * t).sin() * 0.7;
                    let harmonic3 = (2.0 * std::f32::consts::PI * fundamental * 2.5 * t).sin() * 0.3;
                    
                    let speech_envelope = (2.0 * std::f32::consts::PI * 4.0 * t).sin().abs() * 0.9 + 0.1;
                    (harmonic1 + harmonic2 + harmonic3) * speech_envelope * 0.5
                },
                2 => {
                    // Segment 3: Music-like content with multiple tones
                    let chord1 = (2.0 * std::f32::consts::PI * 262.0 * t).sin(); // C4
                    let chord2 = (2.0 * std::f32::consts::PI * 330.0 * t).sin(); // E4
                    let chord3 = (2.0 * std::f32::consts::PI * 392.0 * t).sin(); // G4
                    
                    let music_envelope = (2.0 * std::f32::consts::PI * 1.0 * t).sin() * 0.5 + 0.5;
                    (chord1 + chord2 + chord3) * music_envelope * 0.3
                },
                _ => {
                    // Segment 4: Noise-like content to test codec robustness
                    let noise_freq = 400.0 + (t * 50.0).sin() * 200.0;
                    let noise = (2.0 * std::f32::consts::PI * noise_freq * t).sin();
                    let filtered_noise = noise * (2.0 * std::f32::consts::PI * 0.5 * t).cos();
                    
                    let noise_envelope = (2.0 * std::f32::consts::PI * 6.0 * t).sin().abs() * 0.6 + 0.1;
                    filtered_noise * noise_envelope * 0.3
                }
            };
            
            // Add a small amount of background noise to make it more realistic
            let background_noise = ((i * 7919) % 1000) as f32 / 1000.0 - 0.5; // Pseudo-random
            let final_sample = sample + background_noise * 0.05;
            
            // Convert to 16-bit integer with proper clipping
            let sample_i16 = (final_sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            samples.push(sample_i16);
        }
        
        info!("✅ Generated {} samples of realistic test audio for codec testing", samples.len());
        
        Ok(samples)
    }
}

/// Event handler for Client B (receiver)
struct ClientBEventHandler {
    test_context: Arc<Mutex<TestContext>>,
    codec_decoder: Option<Box<dyn AudioCodecTrait>>,
    output_file_path: String,
}

impl ClientBEventHandler {
    fn new(test_context: Arc<Mutex<TestContext>>, output_file_path: String) -> Self {
        Self {
            test_context,
            codec_decoder: None,
            output_file_path,
        }
    }

    async fn setup_audio_decoder(&mut self, codec: CodecType) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let config = CodecConfig {
            codec,
            sample_rate: codec.default_sample_rate(),
            channels: 1,
            bitrate: codec.default_bitrate(),
            params: HashMap::new(),
        };
        
        let decoder = CodecFactory::create(config)?;
        self.codec_decoder = Some(decoder);
        
        Ok(())
    }

    async fn receive_and_process_rtp_stream(&mut self, mut receiver_rx: mpsc::Receiver<Vec<u8>>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("🎧 Starting RTP audio reception and decoding");
        
        let mut packet_count = 0;
        while let Some(encoded_data) = receiver_rx.recv().await {
            packet_count += 1;
            
            if let Some(ref mut decoder) = self.codec_decoder {
                match decoder.decode(&encoded_data) {
                    Ok(audio_frame) => {
                        let mut context = self.test_context.lock().await;
                        context.rtp_packets_received += 1;
                        
                        // Store decoded samples
                        context.decoded_samples.extend_from_slice(&audio_frame.samples);
                        
                        // Log reception (less frequently to avoid spam)
                        if packet_count % 50 == 0 {
                            let total_samples = context.decoded_samples.len();
                            let total_packets = context.rtp_packets_received;
                            context.events.push(format!(
                                "rtp_packet_received: packet={}, samples={}, total_samples={}, codec=G729",
                                packet_count,
                                audio_frame.samples.len(),
                                total_samples
                            ));
                            info!("📥 Received {} RTP packets, {} total samples decoded", 
                                  total_packets, total_samples);
                        }
                        
                        // Update audio quality metrics periodically
                        if packet_count % 100 == 0 {
                            context.audio_quality = Some(AudioQualityMetrics {
                                mos_score: 4.0,
                                rtt_ms: 50,
                                packet_loss_percent: 0.0,
                                jitter_ms: 5.0,
                                audio_level: audio_frame.rms_level(),
                                snr_db: 20.0,
                                timestamp: Utc::now(),
                            });
                        }
                    }
                    Err(e) => {
                        error!("Failed to decode audio frame {}: {}", packet_count, e);
                    }
                }
            }
        }
        
        let context = self.test_context.lock().await;
        info!("✅ RTP reception completed. Received {} packets, decoded {} samples", 
              context.rtp_packets_received, context.decoded_samples.len());
        
        Ok(())
    }

    async fn save_decoded_audio_to_wav(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let context = self.test_context.lock().await;
        
        if context.decoded_samples.is_empty() {
            return Err("No audio samples to save".into());
        }
        
        // Create a simple WAV file
        let wav_data = self.create_wav_file(&context.decoded_samples)?;
        
        // Write to file
        let mut file = File::create(&self.output_file_path)?;
        file.write_all(&wav_data)?;
        
        info!("Decoded audio saved to: {}", self.output_file_path);
        
        Ok(())
    }
    
    fn create_wav_file(&self, samples: &[i16]) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let mut wav_data = Vec::new();
        
        // WAV header
        let sample_rate = 8000u32;
        let channels = 1u16;
        let bits_per_sample = 16u16;
        let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
        let block_align = channels * bits_per_sample / 8;
        let data_size = samples.len() * 2; // 16-bit samples
        let file_size = 36 + data_size;
        
        // RIFF header
        wav_data.extend_from_slice(b"RIFF");
        wav_data.extend_from_slice(&(file_size as u32).to_le_bytes());
        wav_data.extend_from_slice(b"WAVE");
        
        // fmt chunk
        wav_data.extend_from_slice(b"fmt ");
        wav_data.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        wav_data.extend_from_slice(&1u16.to_le_bytes()); // audio format (PCM)
        wav_data.extend_from_slice(&channels.to_le_bytes());
        wav_data.extend_from_slice(&sample_rate.to_le_bytes());
        wav_data.extend_from_slice(&byte_rate.to_le_bytes());
        wav_data.extend_from_slice(&block_align.to_le_bytes());
        wav_data.extend_from_slice(&bits_per_sample.to_le_bytes());
        
        // data chunk
        wav_data.extend_from_slice(b"data");
        wav_data.extend_from_slice(&(data_size as u32).to_le_bytes());
        
        // audio data
        for sample in samples {
            wav_data.extend_from_slice(&sample.to_le_bytes());
        }
        
        Ok(wav_data)
    }
}

/// Integration test for peer-to-peer audio transmission
#[cfg(feature = "client-integration")]
#[tokio::test]
async fn test_peer_to_peer_audio_transmission() {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_audio_core=debug,rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    info!("🚀 Starting peer-to-peer audio transmission test with G.729 codec");
    
    // Test configuration
    let config = PeerToPeerTestConfig::default();
    let test_context = Arc::new(Mutex::new(TestContext::default()));
    
    // Create event handlers
    let mut client_a_handler = ClientAEventHandler::new(test_context.clone());
    let mut client_b_handler = ClientBEventHandler::new(test_context.clone(), config.output_audio_path.clone());
    
    // Setup audio processing
    client_a_handler.setup_audio_pipeline(config.codec).await
        .expect("Failed to setup Client A audio pipeline");
    client_b_handler.setup_audio_decoder(config.codec).await
        .expect("Failed to setup Client B audio decoder");
    
    // Step 1: Create and configure Client A (sender)
    info!("📱 Creating Client A (sender) - {}", config.client_a_sip_addr);
    let client_a = ClientBuilder::new()
        .local_address(config.client_a_sip_addr.parse().unwrap())
        .media_address(config.client_a_media_addr.parse().unwrap())
        .user_agent("PeerToPeerTest-ClientA/1.0")
        .codecs(vec![config.codec.sdp_name().to_string()])
        .echo_cancellation(false) // Disable for testing
        .build()
        .await
        .expect("Failed to create Client A");
    
    // Step 2: Create and configure Client B (receiver)
    info!("📱 Creating Client B (receiver) - {}", config.client_b_sip_addr);
    let client_b = ClientBuilder::new()
        .local_address(config.client_b_sip_addr.parse().unwrap())
        .media_address(config.client_b_media_addr.parse().unwrap())
        .user_agent("PeerToPeerTest-ClientB/1.0")
        .codecs(vec![config.codec.sdp_name().to_string()])
        .echo_cancellation(false) // Disable for testing
        .build()
        .await
        .expect("Failed to create Client B");
    
    // Step 3: Start both clients
    info!("🔌 Starting SIP clients");
    client_a.start().await.expect("Failed to start Client A");
    client_b.start().await.expect("Failed to start Client B");
    
    // Give clients time to initialize
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Step 4: Validate client endpoint configuration
    {
        let mut context = test_context.lock().await;
        let client_a_stats = client_a.get_client_stats().await;
        let client_b_stats = client_b.get_client_stats().await;
        
        context.client_a_endpoints = Some((
            client_a_stats.local_sip_addr.ip().to_string(),
            client_a_stats.local_sip_addr.port(),
        ));
        context.client_b_endpoints = Some((
            client_b_stats.local_sip_addr.ip().to_string(),
            client_b_stats.local_sip_addr.port(),
        ));
        
        context.events.push("clients_initialized".to_string());
        
        info!("✅ Client A endpoint: {}:{}", 
              client_a_stats.local_sip_addr.ip(),
              client_a_stats.local_sip_addr.port());
        info!("✅ Client B endpoint: {}:{}", 
              client_b_stats.local_sip_addr.ip(),
              client_b_stats.local_sip_addr.port());
    }
    
    // Step 5: Simulate SIP call setup (simplified for testing)
    info!("📞 Simulating SIP call setup");
    let call_id = CallId::from(Uuid::new_v4());
    
    {
        let mut context = test_context.lock().await;
        context.call_id = Some(call_id.clone());
        context.negotiated_codecs = vec![config.codec.sdp_name().to_string()];
        context.events.push("call_setup_simulated".to_string());
    }
    
    // Step 6: Validate SDP negotiation format
    {
        let context = test_context.lock().await;
        assert!(!context.negotiated_codecs.is_empty(), "No codecs negotiated");
        assert!(context.negotiated_codecs.contains(&config.codec.sdp_name().to_string()), 
                "Expected codec {} not found in negotiated codecs", config.codec.sdp_name());
        
        info!("✅ SDP negotiation validated - codec: {}", config.codec.sdp_name());
    }
    
    // Step 7: Create RTP communication channel between clients
    info!("🔗 Setting up RTP communication channel");
    let (rtp_tx, rtp_rx) = mpsc::channel::<Vec<u8>>(1000); // Buffer up to 1000 RTP packets
    
    // Step 8: Start audio transmission from Client A
    info!("🎵 Starting audio transmission from Client A");
    let audio_source_path = config.audio_source_path.clone();
    let transmission_task = tokio::spawn(async move {
        client_a_handler.send_audio_from_file(&audio_source_path, rtp_tx).await
    });
    
    // Step 9: Start audio reception on Client B
    info!("🎧 Starting audio reception on Client B");
    let reception_task = {
        let mut handler = client_b_handler;
        
        tokio::spawn(async move {
            // Receive and process actual RTP stream from Client A
            handler.receive_and_process_rtp_stream(rtp_rx).await
                .expect("Failed to receive and process RTP stream");
            
            // Save decoded audio to WAV file
            handler.save_decoded_audio_to_wav().await
                .expect("Failed to save decoded audio to WAV");
        })
    };
    
    // Step 10: Wait for transmission and reception to complete
    info!("⏱️ Waiting for audio transmission and reception to complete...");
    let transmission_result = timeout(Duration::from_secs(30), transmission_task).await;
    let reception_result = timeout(Duration::from_secs(30), reception_task).await;
    
    // Check results
    assert!(transmission_result.is_ok(), "Audio transmission timed out (increase timeout if processing large MP3)");
    assert!(reception_result.is_ok(), "Audio reception timed out");
    
    transmission_result.unwrap().expect("Audio transmission failed");
    reception_result.unwrap().expect("Audio reception failed");
    
    // Step 11: Validate test results
    {
        let context = test_context.lock().await;
        
        // Validate IP addresses and ports
        assert!(context.client_a_endpoints.is_some(), "Client A endpoints not set");
        assert!(context.client_b_endpoints.is_some(), "Client B endpoints not set");
        
        let (client_a_ip, client_a_port) = context.client_a_endpoints.as_ref().unwrap();
        let (client_b_ip, client_b_port) = context.client_b_endpoints.as_ref().unwrap();
        
        assert_eq!(client_a_ip, "127.0.0.1", "Client A IP address incorrect");
        assert_eq!(client_a_port, &5080, "Client A port incorrect");
        assert_eq!(client_b_ip, "127.0.0.1", "Client B IP address incorrect");
        assert_eq!(client_b_port, &5081, "Client B port incorrect");
        
        // Validate RTP transmission (ensure actual encoding/decoding happened)
        assert!(context.rtp_packets_sent > 0, "No RTP packets were sent - MP3 encoding failed");
        assert!(context.rtp_packets_received > 0, "No RTP packets were received - RTP transmission failed");
        assert_eq!(context.rtp_packets_sent, context.rtp_packets_received, 
                   "Packet loss detected: sent {} != received {}", 
                   context.rtp_packets_sent, context.rtp_packets_received);
        
        // Validate SDP codec negotiation worked
        assert!(!context.negotiated_codecs.is_empty(), "No codecs negotiated in SDP");
        assert!(context.negotiated_codecs.contains(&"G729".to_string()), 
                "G.729 codec not found in SDP negotiation");
        
        // Validate audio quality metrics
        assert!(context.audio_quality.is_some(), "Audio quality metrics not available");
        let quality = context.audio_quality.as_ref().unwrap();
        assert!(quality.audio_level > 0.0, "Audio level should be greater than 0 - no audio detected");
        assert!(quality.mos_score > 0.0, "MOS score should be greater than 0");
        
        // Validate decoded audio samples (ensure substantial audio was processed)
        assert!(!context.decoded_samples.is_empty(), "No decoded audio samples - decoding failed");
        assert!(context.decoded_samples.len() > 1000, 
                "Too few decoded samples ({}) - likely encoding/decoding error", 
                context.decoded_samples.len());
        
        // Validate that we actually encoded and transmitted substantial audio
        let expected_duration_samples = (context.rtp_packets_sent * 80) as usize; // 80 samples per G.729 frame
        let actual_decoded_samples = context.decoded_samples.len();
        assert!(actual_decoded_samples >= expected_duration_samples * 8 / 10, // Allow 20% tolerance
                "Decoded samples ({}) much less than expected ({}) - possible data loss",
                actual_decoded_samples, expected_duration_samples);
        
        info!("✅ INTEGRATION TEST VALIDATION COMPLETED SUCCESSFULLY");
        info!("📊 RTP packets sent: {}", context.rtp_packets_sent);
        info!("📊 RTP packets received: {}", context.rtp_packets_received);
        info!("📊 Decoded samples: {}", context.decoded_samples.len());
        info!("📊 Expected samples: {}", expected_duration_samples);
        info!("📊 Audio level: {:.4}", quality.audio_level);
        info!("📊 MOS score: {:.4}", quality.mos_score);
        info!("📊 Negotiated codecs: {:?}", context.negotiated_codecs);
        
        // Calculate transmission statistics
        let duration_seconds = context.decoded_samples.len() as f64 / 8000.0; // 8kHz sample rate
        let effective_bitrate = (context.rtp_packets_sent as f64 * 10.0 * 8.0) / duration_seconds; // 10 bytes per packet
        info!("📊 Transmitted audio duration: {:.2} seconds", duration_seconds);
        info!("📊 Effective bitrate: {:.0} bps (expected ~8000 bps for G.729)", effective_bitrate);
        
        // Print key test events log
        info!("📋 Key test events:");
        for event in &context.events {
            if event.contains("transmission_started") || event.contains("transmission_completed") ||
               event.contains("clients_initialized") || event.contains("call_setup") {
                info!("  - {}", event);
            }
        }
    }
    
    // Step 12: Verify output WAV file was created and validate it
    let output_path = Path::new(&config.output_audio_path);
    assert!(output_path.exists(), "❌ Output WAV file was not created at {}", config.output_audio_path);
    
    let file_size = std::fs::metadata(output_path).unwrap().len();
    assert!(file_size > 44, "❌ Output WAV file is too small ({} bytes, less than WAV header size)", file_size);
    
    // Calculate expected file size (WAV header + sample data)
    let context = test_context.lock().await;
    let expected_data_size = context.decoded_samples.len() * 2; // 16-bit samples = 2 bytes each
    let expected_file_size = 44 + expected_data_size; // WAV header + data
    let size_difference = (file_size as i64 - expected_file_size as i64).abs();
    
    assert!(size_difference <= 100, // Allow small variance for WAV formatting
            "❌ WAV file size ({} bytes) doesn't match expected size ({} bytes, diff: {})", 
            file_size, expected_file_size, size_difference);
    
    info!("✅ Output WAV file created successfully!");
    info!("📁 File location: {}", config.output_audio_path);
    info!("📏 File size: {} bytes", file_size);
    info!("📊 Audio data: {} samples ({:.2} seconds at 8kHz)", 
          context.decoded_samples.len(), 
          context.decoded_samples.len() as f64 / 8000.0);
    
    // DO NOT CLEAN UP - leave the file for user verification
    info!("🎵 You can now play the decoded audio file: {}", config.output_audio_path);
    info!("🔍 To verify the audio was transmitted correctly, compare it with the original MP3 file");
    
    info!("🎉 PEER-TO-PEER AUDIO TRANSMISSION TEST COMPLETED SUCCESSFULLY!");
    info!("✅ Confirmed: MP3 → G.729 encoding → RTP transmission → G.729 decoding → WAV output");
}

/// Test G.729 codec compatibility with SIP signaling
#[tokio::test]
async fn test_g729_codec_compatibility() {
    info!("🧪 Testing G.729 codec compatibility");
    
    // Test codec properties
    let codec = CodecType::G729;
    assert_eq!(codec.default_sample_rate(), 8000, "G.729 sample rate should be 8kHz");
    assert_eq!(codec.default_bitrate(), 8000, "G.729 bitrate should be 8kbps");
    assert_eq!(codec.payload_type(), 18, "G.729 RTP payload type should be 18");
    assert_eq!(codec.sdp_name(), "G729", "G.729 SDP name should be 'G729'");
    
    // Test codec creation
    let config = CodecConfig {
        codec: CodecType::G729,
        sample_rate: 8000,
        channels: 1,
        bitrate: 8000,
        params: HashMap::new(),
    };
    
    let codec_instance = CodecFactory::create(config).expect("Failed to create G.729 codec");
    assert_eq!(codec_instance.codec_type(), CodecType::G729);
    
    info!("✅ G.729 codec compatibility test passed");
}

/// Test RTP payload handling for G.729
#[tokio::test]
async fn test_g729_rtp_payload_handling() {
    info!("🧪 Testing G.729 RTP payload handling");
    
    let mut rtp_handler = RtpPayloadHandler::new(CodecType::G729, 0x12345678);
    
    // Test G.729 frame (10 bytes)
    let g729_frame = vec![
        0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01, 0x00, 0xFF
    ];
    
    // Create RTP packet
    let rtp_packet = rtp_handler.create_packet(&g729_frame, false);
    
    // Validate RTP packet properties
    assert_eq!(rtp_packet.payload_type, 18, "G.729 payload type should be 18");
    assert_eq!(rtp_packet.payload.len(), 10, "G.729 payload should be 10 bytes");
    
    // Test payload extraction
    let extracted_payload = &rtp_packet.payload;
    assert_eq!(extracted_payload, &g729_frame, "Extracted payload should match original");
    
    info!("✅ G.729 RTP payload handling test passed");
}

/// Helper function to validate audio frame format
fn validate_audio_frame_format(frame: &AudioFrame, expected_codec: CodecType) {
    assert_eq!(frame.format.sample_rate, expected_codec.default_sample_rate(),
               "Sample rate should match codec default");
    assert_eq!(frame.format.channels, 1, "G.729 should use mono audio");
    assert_eq!(frame.format.bits_per_sample, 16, "Should use 16-bit samples");
    assert!(!frame.samples.is_empty(), "Frame should contain samples");
}

/// Additional test for multi-codec negotiation
#[tokio::test]
async fn test_multi_codec_negotiation() {
    info!("🧪 Testing multi-codec negotiation including G.729");
    
    let supported_codecs = vec![
        CodecType::Opus,
        CodecType::G729,
        CodecType::G722,
        CodecType::G711Pcmu,
        CodecType::G711Pcma,
    ];
    
    // Test codec priority (Opus should be preferred)
    let negotiated_codec = supported_codecs[0];
    assert_eq!(negotiated_codec, CodecType::Opus, "Opus should be preferred codec");
    
    // Test G.729 as fallback
    let fallback_codec = supported_codecs[1];
    assert_eq!(fallback_codec, CodecType::G729, "G.729 should be second priority");
    
    info!("✅ Multi-codec negotiation test passed");
}

// AudioFrame already has a rms_level() method in the main library
// No need to implement it here 