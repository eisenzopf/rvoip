use std::time::Duration;
use std::io::stdin;

use anyhow::Result;
use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::time::sleep;
use tracing::{debug, info, warn, error};

use rvoip_rtp_core::{RtpPacket, RtpSession, RtpSessionConfig, RtpTimestamp};
use rvoip_media_core::codec::{Codec, G711Codec, G711Variant};
use rvoip_media_core::{AudioBuffer, AudioFormat, SampleRate};

/// Generate a simple tone as a PCM audio buffer
fn generate_tone(frequency: f32, duration_ms: u32, sample_rate: u32) -> AudioBuffer {
    let num_samples = (duration_ms as f32 * sample_rate as f32 / 1000.0) as usize;
    let mut pcm_data = BytesMut::with_capacity(num_samples * 2); // 16-bit samples
    
    for i in 0..num_samples {
        // Generate sine wave
        let t = i as f32 / sample_rate as f32;
        let sample = (std::f32::consts::PI * 2.0 * frequency * t).sin() * 16384.0; // 50% amplitude
        let sample = sample as i16;
        
        // Add sample in little-endian order
        pcm_data.extend_from_slice(&[(sample & 0xFF) as u8, ((sample >> 8) & 0xFF) as u8]);
    }
    
    AudioBuffer::new(
        pcm_data.freeze(),
        AudioFormat::mono_16bit(SampleRate::from_hz(sample_rate))
    )
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
    
    // Create G.711 codec
    let codec = G711Codec::new(G711Variant::PCMU);
    
    // Start a task to listen for incoming packets
    let decoder = codec.clone();
    
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
                                          decoded.samples(),
                                          decoded.duration_ms());
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
    let audio_buffer = generate_tone(1000.0, 1000, 8000); // 1 second of 1000 Hz tone
    info!("Generated {} ms of 1000 Hz tone", audio_buffer.duration_ms());
    
    // Split into 20ms packets and send
    let samples_per_packet = (20.0 * 8000.0 / 1000.0) as usize; // 20ms at 8kHz = 160 samples
    let bytes_per_sample = 2; // 16-bit PCM
    let bytes_per_packet = samples_per_packet * bytes_per_sample;
    
    let total_packets = (audio_buffer.data.len() + bytes_per_packet - 1) / bytes_per_packet;
    info!("Sending {} packets of 20ms each", total_packets);
    
    // Initialize RTP timestamp (8kHz clock rate)
    let mut timestamp: RtpTimestamp = 0;
    
    for i in 0..total_packets {
        let start = i * bytes_per_packet;
        let end = std::cmp::min(start + bytes_per_packet, audio_buffer.data.len());
        
        // Extract 20ms chunk
        let chunk = AudioBuffer::new(
            audio_buffer.data.slice(start..end),
            audio_buffer.format
        );
        
        // Encode with G.711
        match codec.encode(&chunk) {
            Ok(encoded) => {
                // Send via RTP
                if let Err(e) = session.send_packet(timestamp, encoded, false).await {
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