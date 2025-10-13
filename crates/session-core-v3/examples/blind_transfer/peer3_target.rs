//! Peer3 (Charlie) - Receives the transferred call from Peer1 (Alice)

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

    println!("\n[CHARLIE] Starting - Will receive transferred call from Alice...");

    // Configure Charlie (Peer3)
    let config = Config {
        sip_port: 5062,
        media_port_start: 10200,
        media_port_end: 10300,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5062".parse()?,
        state_table_path: None,
        local_uri: "sip:charlie@127.0.0.1:5062".to_string(),
    };

    let mut charlie = SimplePeer::with_config("charlie", config).await?;
    println!("[CHARLIE] ✅ Listening on port 5062...");

    // Wait for the transferred call from Alice (via Bob's REFER)
    println!("[CHARLIE] ⏳ Waiting for transferred call from Alice...");
    let incoming = charlie.wait_for_call().await?;

    info!("[CHARLIE] Incoming call from: {} (ID: {})", incoming.from, incoming.id);
    println!("[CHARLIE] 📞 Received transferred call!");

    // Accept the call
    println!("[CHARLIE] 📞 Accepting call...");
    charlie.accept(&incoming.id).await?;

    // Subscribe to receive audio
    let mut audio_rx = charlie.subscribe_audio(&incoming.id).await?;

    // Storage for sent and received audio
    let mut sent_samples: Vec<i16> = Vec::new();
    let mut received_samples: Vec<i16> = Vec::new();

    // Wait for call to fully establish
    sleep(Duration::from_secs(1)).await;

    // Now talking to Alice - send 659Hz tone while receiving simultaneously
    println!("[CHARLIE] 💬 Now talking to Alice (post-transfer, sending 659Hz tone)...");
    let sample_rate = 8000u32;
    let duration_ms = 20u32;
    let samples_per_frame = (sample_rate * duration_ms / 1000) as usize;

    // Spawn receiving task to run concurrently with sending
    let receive_task = tokio::spawn(async move {
        let mut samples = Vec::new();
        let start_time = std::time::Instant::now();
        let receive_timeout = Duration::from_secs(10);

        while start_time.elapsed() < receive_timeout {
            match tokio::time::timeout(Duration::from_millis(100), audio_rx.recv()).await {
                Ok(Some(frame)) => {
                    samples.extend_from_slice(&frame.samples);
                }
                Ok(None) => {
                    println!("[CHARLIE] Audio channel closed");
                    break;
                }
                Err(_) => {
                    // Timeout, continue
                }
            }
        }
        samples
    });

    // Send audio for 10 seconds (500 frames) while receiving
    for i in 0u32..500 {
        let mut samples = Vec::with_capacity(samples_per_frame);
        for j in 0..samples_per_frame {
            let t = ((i as usize * samples_per_frame + j) as f32) / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * 659.0 * t).sin();
            samples.push((sample * 16384.0) as i16);
        }

        sent_samples.extend_from_slice(&samples);
        let frame = AudioFrame::new(samples, sample_rate, 1, i * duration_ms);
        charlie.send_audio(&incoming.id, frame).await?;
        sleep(Duration::from_millis(20)).await;
    }

    println!("[CHARLIE] Sent {} audio samples to Alice", sent_samples.len());

    // Wait for receiving task to complete
    println!("[CHARLIE] Waiting for received audio...");
    received_samples = receive_task.await.unwrap_or_default();

    println!("[CHARLIE] Received {} samples from Alice", received_samples.len());
    println!("[CHARLIE] Total: sent {} samples, received {} samples",
             sent_samples.len(), received_samples.len());

    // Save audio files
    save_audio_files("charlie", &sent_samples, &received_samples)?;

    // Hang up the call
    println!("[CHARLIE] 📴 Hanging up...");
    charlie.hangup(&incoming.id).await?;

    println!("[CHARLIE] ✅ Test complete!");

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
