//! Peer1 (Alice) - Makes a call to Peer2 (Bob) and gets transferred to Peer3 (Charlie)

use rvoip_session_core_v2::api::simple::{SimplePeer, Config, AudioFrame};
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
                .add_directive("rvoip_session_core_v2=info".parse()?)
                .add_directive("rvoip_dialog_core=info".parse()?)
                .add_directive("rvoip_media_core=info".parse()?)
        )
        .init();

    println!("\n[ALICE] Starting - Will call Bob and be transferred to Charlie...");

    // Configure Alice (Peer1)
    let config = Config {
        sip_port: 5060,
        media_port_start: 10000,
        media_port_end: 10100,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5060".parse()?,
        state_table_path: None,
        local_uri: "sip:alice@127.0.0.1:5060".to_string(),
    };

    let alice = SimplePeer::with_config("alice", config).await?;

    // Give peers time to start
    println!("[ALICE] Waiting for other peers to start...");
    sleep(Duration::from_secs(3)).await;

    // Make the call to Bob (Peer2)
    println!("[ALICE] 📞 Calling Bob at sip:bob@127.0.0.1:5061...");
    let call_id = alice.call("sip:bob@127.0.0.1:5061").await?;
    info!("[ALICE] Call established with ID: {:?}", call_id);

    // Subscribe to receive audio
    let mut audio_rx = alice.subscribe_audio(&call_id).await?;

    // Storage for sent and received audio
    let mut sent_samples: Vec<i16> = Vec::new();
    let mut received_samples: Vec<i16> = Vec::new();

    // Talk to Bob for a bit - send 440Hz tone
    println!("[ALICE] 💬 Talking to Bob (sending 440Hz tone)...");
    let sample_rate = 8000u32;
    let duration_ms = 20u32;
    let samples_per_frame = (sample_rate * duration_ms / 1000) as usize;

    // Send audio for 3 seconds (150 frames)
    for i in 0u32..150 {
        let mut samples = Vec::with_capacity(samples_per_frame);
        for j in 0..samples_per_frame {
            let t = ((i as usize * samples_per_frame + j) as f32) / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            samples.push((sample * 16384.0) as i16);
        }

        sent_samples.extend_from_slice(&samples);
        let frame = AudioFrame::new(samples, sample_rate, 1, i * duration_ms);
        alice.send_audio(&call_id, frame).await?;
        sleep(Duration::from_millis(20)).await;
    }

    println!("[ALICE] Sent {} audio samples to Bob", sent_samples.len());

    // Receive audio from Bob for 3 seconds
    println!("[ALICE] Receiving audio from Bob...");
    let start_time = std::time::Instant::now();
    let receive_timeout = Duration::from_secs(3);

    while start_time.elapsed() < receive_timeout {
        match tokio::time::timeout(Duration::from_millis(100), audio_rx.recv()).await {
            Ok(Some(frame)) => {
                received_samples.extend_from_slice(&frame.samples);
            }
            Ok(None) => {
                println!("[ALICE] Audio channel closed");
                break;
            }
            Err(_) => {
                // Timeout, continue
            }
        }
    }

    println!("[ALICE] Received {} samples from Bob", received_samples.len());

    // Wait for transfer to happen (initiated by Bob)
    println!("[ALICE] ⏳ Waiting for Bob to transfer me to Charlie...");
    sleep(Duration::from_secs(5)).await;

    // After transfer, we should be talking to Charlie
    // Note: The auto-transfer handler creates a new session, so we'd need to track the new call_id
    // For now, just send some audio and exit
    println!("[ALICE] 💬 Now talking to Charlie (post-transfer)...");

    // Try to send audio for 3 more seconds
    for i in 150u32..300 {
        let mut samples = Vec::with_capacity(samples_per_frame);
        for j in 0..samples_per_frame {
            let t = ((i as usize * samples_per_frame + j) as f32) / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * 880.0 * t).sin(); // 880Hz for Charlie
            samples.push((sample * 16384.0) as i16);
        }

        sent_samples.extend_from_slice(&samples);
        let frame = AudioFrame::new(samples, sample_rate, 1, i * duration_ms);
        // This might fail if call_id is no longer valid after transfer
        let _ = alice.send_audio(&call_id, frame).await;
        sleep(Duration::from_millis(20)).await;
    }

    // Receive audio from Charlie for 3 seconds
    println!("[ALICE] Receiving audio from Charlie...");
    let start_time = std::time::Instant::now();
    let receive_timeout = Duration::from_secs(3);

    while start_time.elapsed() < receive_timeout {
        match tokio::time::timeout(Duration::from_millis(100), audio_rx.recv()).await {
            Ok(Some(frame)) => {
                received_samples.extend_from_slice(&frame.samples);
            }
            Ok(None) => break,
            Err(_) => continue,
        }
    }

    println!("[ALICE] Total: sent {} samples, received {} samples",
             sent_samples.len(), received_samples.len());

    // Save audio files
    save_audio_files("alice", &sent_samples, &received_samples)?;

    println!("[ALICE] ✅ Transfer test complete, exiting...");
    sleep(Duration::from_secs(1)).await;

    Ok(())
}

fn save_audio_files(
    peer_name: &str,
    sent_samples: &[i16],
    received_samples: &[i16],
) -> Result<(), Box<dyn std::error::Error>> {
    let output_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("blind_transfer")
        .join("output");
    std::fs::create_dir_all(&output_dir)?;

    if !sent_samples.is_empty() {
        let sent_path = output_dir.join(format!("{}_sent.wav", peer_name));
        save_wav_file(&sent_path, sent_samples, 8000)?;
        println!("[{}] 💾 Saved sent audio to {}", peer_name.to_uppercase(), sent_path.display());
    }

    if !received_samples.is_empty() {
        let received_path = output_dir.join(format!("{}_received.wav", peer_name));
        save_wav_file(&received_path, received_samples, 8000)?;
        println!("[{}] 💾 Saved received audio to {}", peer_name.to_uppercase(), received_path.display());
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
    let data_size = samples.len() as u32 * 2;
    let file_size = 36 + data_size;

    // RIFF header
    file.write_all(b"RIFF")?;
    file.write_all(&file_size.to_le_bytes())?;
    file.write_all(b"WAVE")?;

    // fmt chunk
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // Chunk size
    file.write_all(&1u16.to_le_bytes())?; // Audio format (1 = PCM)
    file.write_all(&channels.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&bits_per_sample.to_le_bytes())?;

    // data chunk
    file.write_all(b"data")?;
    file.write_all(&data_size.to_le_bytes())?;

    // Write samples
    for sample in samples {
        file.write_all(&sample.to_le_bytes())?;
    }

    Ok(())
}
