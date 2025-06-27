use std::time::Duration;
use std::io::stdin;

use anyhow::Result;
use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::time::sleep;
use tracing::{debug, info, warn, error};

use rvoip_rtp_core::{RtpPacket, RtpSession, RtpSessionConfig, RtpTimestamp};
use rvoip_media_core::codec::AudioCodec;
use rvoip_media_core::prelude::{G711Codec, G711Variant, G711Config};
use rvoip_media_core::{AudioBuffer, AudioFormat, SampleRate, AudioFrame};

/// Generate a simple tone as a PCM audio buffer
fn generate_tone(frequency: f32, duration_ms: u32, sample_rate: u32) -> Vec<i16> {
    let num_samples = (duration_ms as f32 * sample_rate as f32 / 1000.0) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    
    for i in 0..num_samples {
        // Generate sine wave
        let t = i as f32 / sample_rate as f32;
        let sample = (std::f32::consts::PI * 2.0 * frequency * t).sin() * 16384.0; // 50% amplitude
        samples.push(sample as i16);
    }
    
    samples
}

/// Demo application that sends a tone via RTP with G.711 encoding
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("Starting G.711 over RTP demo");
    
    // Get local address to use
    let local_addr = "127.0.0.1:5000".parse().unwrap();
    let remote_addr = "127.0.0.1:5001".parse().unwrap();
    
    println!("Setting up local RTP endpoint at {}", local_addr);
    println!("Will send to remote endpoint at {}", remote_addr);
    println!("Press Enter to start...");
    let _ = stdin().read_line(&mut String::new())?;
    
    // Create RTP session with G.711 μ-law codec
    let session_config = RtpSessionConfig {
        local_addr,
        remote_addr: Some(remote_addr),
        payload_type: 0, // 0 = PCMU
        clock_rate: 8000,
        ..Default::default()
    };
    
    // Initialize the RTP session
    let mut session = RtpSession::new(session_config).await?;
    
    // Create G.711 codec configuration
    let config = G711Config {
        variant: G711Variant::MuLaw,
        sample_rate: 8000,
        channels: 1,
        frame_size_ms: 20.0,
    };
    
    // Create G.711 codec
    let mut codec = G711Codec::new(
        SampleRate::from_hz(8000).unwrap(),
        1, // channels
        config.clone()
    )?;
    
    // Create decoder for receiving
    let mut decoder = G711Codec::new(
        SampleRate::from_hz(8000).unwrap(),
        1, // channels
        config
    )?;
    
    // Create socket for receiving
    let recv_socket = UdpSocket::bind("127.0.0.1:5001").await?;
    
    tokio::spawn(async move {
        let mut buf = vec![0u8; 1500];
        loop {
            match recv_socket.recv(&mut buf).await {
                Ok(len) => {
                    match RtpPacket::parse(&buf[..len]) {
                        Ok(packet) => {
                            // Decode G.711 audio
                            match decoder.decode(&packet.payload) {
                                Ok(decoded) => {
                                    info!("Received {} samples ({} ms) of audio",
                                          decoded.samples.len(),
                                          decoded.samples.len() as f32 * 1000.0 / 8000.0);
                                },
                                Err(e) => {
                                    warn!("Failed to decode G.711 audio: {}", e);
                                }
                            }
                        },
                        Err(e) => {
                            warn!("Failed to parse RTP packet: {}", e);
                        }
                    }
                },
                Err(e) => {
                    error!("UDP receive error: {}", e);
                    break;
                }
            }
        }
    });
    
    // Generate a 1000 Hz tone (20ms per packet, 8000 Hz sample rate)
    println!("Generating and sending a 1000 Hz tone...");
    
    // Generate several packets worth of audio
    let tone_samples = generate_tone(1000.0, 1000, 8000); // 1 second of 1000 Hz tone
    info!("Generated {} samples ({} ms) of 1000 Hz tone", 
          tone_samples.len(), 
          tone_samples.len() as f32 * 1000.0 / 8000.0);
    
    // Split into 20ms packets and send
    let samples_per_packet = (20.0 * 8000.0 / 1000.0) as usize; // 20ms at 8kHz = 160 samples
    
    let total_packets = (tone_samples.len() + samples_per_packet - 1) / samples_per_packet;
    info!("Sending {} packets of 20ms each", total_packets);
    
    // Initialize RTP timestamp (8kHz clock rate)
    let mut timestamp: RtpTimestamp = 0;
    
    for i in 0..total_packets {
        let start = i * samples_per_packet;
        let end = std::cmp::min(start + samples_per_packet, tone_samples.len());
        
        // Extract 20ms chunk
        let chunk_samples = &tone_samples[start..end];
        
        // Create AudioFrame for encoding
        let audio_frame = AudioFrame::new(
            chunk_samples.to_vec(),
            8000, // sample rate
            1,    // channels
            timestamp
        );
        
        // Encode with G.711
        match codec.encode(&audio_frame) {
            Ok(encoded) => {
                // Send via RTP
                if let Err(e) = session.send_packet(timestamp, encoded.into(), false).await {
                    warn!("Failed to send RTP packet: {}", e);
                } else {
                    debug!("Sent packet {} with timestamp {}", i + 1, timestamp);
                }
                
                // Increment timestamp by number of samples
                timestamp = timestamp.wrapping_add(samples_per_packet as u32);
            },
            Err(e) => {
                warn!("Failed to encode audio chunk: {}", e);
            }
        }
        
        // Sleep to simulate real-time sending (20ms per packet)
        sleep(Duration::from_millis(20)).await;
    }
    
    println!("Done sending audio. Wait 1 second for any remaining packets...");
    sleep(Duration::from_secs(1)).await;
    
    // Close the session
    session.close().await;
    
    info!("G.711 over RTP demo completed");
    Ok(())
} 