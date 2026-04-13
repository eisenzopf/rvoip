//! Bob - Receives a call and sends audio

use rvoip_session_core_v3::{StreamPeer, Config};
use rvoip_media_core::types::AudioFrame;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use tokio::time::{sleep, Duration};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Simple logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_session_core_v3=info".parse()?)
                .add_directive("rvoip_dialog_core=info".parse()?)
                .add_directive("rvoip_media_core=info".parse()?)
        )
        .init();

    println!("[BOB] Starting...");

    // Configure Bob
    let config = Config {
        sip_port: 5061,
        media_port_start: 10100,
        media_port_end: 10200,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5061".parse()?,
        state_table_path: None,
        local_uri: "sip:bob@127.0.0.1:5061".to_string(),
    };

    let mut bob = StreamPeer::with_config(config).await?;

    // Wait for incoming call
    println!("[BOB] Waiting for incoming call...");

    let incoming = bob.wait_for_incoming().await?;
    println!("[BOB] Incoming call from: {}", incoming.from);

    // Accept the call
    println!("[BOB] Accepting call...");
    let handle = incoming.accept().await?;
    info!("Accepted call with ID: {:?}", handle.id());

    // Wait for call to be fully established
    sleep(Duration::from_secs(1)).await;

    // Get audio stream
    let audio = handle.audio().await?;
    let (audio_tx, mut audio_rx) = audio.split();

    // Prepare audio storage
    let mut sent_samples = Vec::new();
    let mut received_samples = Vec::new();

    // Spawn receiver task
    let recv_handle = tokio::spawn(async move {
        let mut received = Vec::new();
        let start = std::time::Instant::now();
        let timeout_dur = Duration::from_secs(8);
        while start.elapsed() < timeout_dur {
            match tokio::time::timeout(Duration::from_millis(100), audio_rx.recv()).await {
                Ok(Some(frame)) => {
                    received.extend_from_slice(&frame.samples);
                    if received.len() % 8000 == 0 {
                        println!("[BOB] Received {} samples so far...", received.len());
                    }
                }
                Ok(None) => {
                    println!("[BOB] Audio channel closed");
                    break;
                }
                Err(_) => {} // timeout, continue
            }
        }
        received
    });

    // Wait a bit before sending to ensure Alice is ready
    sleep(Duration::from_millis(500)).await;

    // Send audio - 5 seconds of 880Hz tone
    println!("[BOB] Sending audio (880Hz tone)...");
    let sample_rate = 8000u32;
    let duration_ms = 20u32;
    let samples_per_frame = (sample_rate * duration_ms / 1000) as usize;

    for i in 0u32..250 {  // 250 frames = 5 seconds
        let mut samples = Vec::with_capacity(samples_per_frame);
        for j in 0..samples_per_frame {
            let t = ((i as usize * samples_per_frame + j) as f32) / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * 880.0 * t).sin();
            samples.push((sample * 16384.0) as i16);
        }

        // Store what we're sending
        sent_samples.extend_from_slice(&samples);

        let frame = AudioFrame::new(
            samples,
            sample_rate,
            1, // channels
            i * duration_ms, // timestamp
        );

        audio_tx.send(frame).await?;
        sleep(Duration::from_millis(duration_ms as u64)).await;
    }

    println!("[BOB] Finished sending {} audio samples", sent_samples.len());

    // Wait for receiver to finish
    received_samples = recv_handle.await?;
    println!("[BOB] Received {} total audio samples", received_samples.len());

    // Save audio files if recording is enabled
    if std::env::var("RECORD_AUDIO").is_ok() {
        save_audio_files("bob", &sent_samples, &received_samples)?;
    }

    // Give Alice time to hang up first
    sleep(Duration::from_secs(2)).await;

    println!("[BOB] Done! Sent {} samples, received {} samples",
             sent_samples.len(), received_samples.len());

    Ok(())
}

fn save_audio_files(
    peer_name: &str,
    sent_samples: &[i16],
    received_samples: &[i16],
) -> Result<(), Box<dyn std::error::Error>> {
    // Create output directory in the example directory
    let output_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("api_peer_audio")
        .join("output");
    std::fs::create_dir_all(&output_dir)?;

    // Save sent audio as WAV file
    if !sent_samples.is_empty() {
        let sent_path = output_dir.join(format!("{}_sent.wav", peer_name));
        save_wav_file(&sent_path, sent_samples, 8000)?;
        println!("[BOB] Saved sent audio to {}", sent_path.display());
    }

    // Save received audio as WAV file
    if !received_samples.is_empty() {
        let received_path = output_dir.join(format!("{}_received.wav", peer_name));
        save_wav_file(&received_path, received_samples, 8000)?;
        println!("[BOB] Saved received audio to {}", received_path.display());
    }

    Ok(())
}

fn save_wav_file(
    path: &Path,
    samples: &[i16],
    sample_rate: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::create(path)?;

    // WAV header
    let channels = 1u16;
    let bits_per_sample = 16u16;
    let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_size = samples.len() as u32 * 2; // 2 bytes per sample
    let file_size = 36 + data_size;

    // Write WAV header
    file.write_all(b"RIFF")?;
    file.write_all(&file_size.to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // fmt chunk size
    file.write_all(&1u16.to_le_bytes())?; // PCM format
    file.write_all(&channels.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&bits_per_sample.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_size.to_le_bytes())?;

    // Write samples
    for sample in samples {
        file.write_all(&sample.to_le_bytes())?;
    }

    Ok(())
}
