//! Bidirectional audio exchange between two peers in a single process.
//!
//!   cargo run --example streampeer_audio
//!
//! Alice calls Bob, they exchange audio tones for 3 seconds, then hang up.
//! Audio is saved to WAV files in an `output/` directory.

use rvoip_media_core::types::AudioFrame;
use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

const SAMPLE_RATE: u32 = 8000;
const FRAME_SIZE: usize = 160; // 20ms at 8kHz

fn generate_tone(freq: f32, frame_num: usize) -> Vec<i16> {
    (0..FRAME_SIZE)
        .map(|j| {
            let t = (frame_num * FRAME_SIZE + j) as f32 / SAMPLE_RATE as f32;
            (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

fn save_wav(name: &str, samples: &[i16]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all("output")?;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(format!("output/{}", name), spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    println!("Saved output/{}", name);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    // Spawn Bob (receiver) as a background task
    let bob_task = tokio::spawn(async {
        let mut bob = StreamPeer::with_config(Config {
            media_port_start: 10100,
            media_port_end: 10200,
            ..Config::local("bob", 5061)
        })
        .await?;

        println!("[BOB] Waiting for call...");
        let incoming = bob.wait_for_incoming().await?;
        println!("[BOB] Call from {}", incoming.from);
        let handle = incoming.accept().await?;

        // Send 880Hz tone and receive audio
        let audio = handle.audio().await?;
        let (sender, mut receiver) = audio.split();

        let recv_task = tokio::spawn(async move {
            let mut samples = Vec::new();
            while let Some(frame) = receiver.recv().await {
                samples.extend_from_slice(&frame.samples);
            }
            samples
        });

        for i in 0..150 {
            // 3 seconds at 20ms/frame
            let samples = generate_tone(880.0, i);
            let frame = AudioFrame::new(samples, SAMPLE_RATE, 1, (i * FRAME_SIZE) as u32);
            if sender.send(frame).await.is_err() {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }

        handle.wait_for_end(Some(Duration::from_secs(5))).await.ok();
        let received = recv_task.await.unwrap_or_default();
        save_wav("bob_received.wav", &received)?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });

    // Give Bob a moment to start listening
    sleep(Duration::from_secs(1)).await;

    // Alice (caller)
    let mut alice = StreamPeer::with_config(Config {
        media_port_start: 10000,
        media_port_end: 10100,
        ..Config::local("alice", 5060)
    })
    .await?;

    println!("[ALICE] Calling Bob...");
    let handle = alice.call("sip:bob@127.0.0.1:5061").await?;
    alice.wait_for_answered(handle.id()).await?;
    println!("[ALICE] Connected!");

    // Send 440Hz tone and receive audio
    let audio = handle.audio().await?;
    let (sender, mut receiver) = audio.split();

    let recv_task = tokio::spawn(async move {
        let mut samples = Vec::new();
        while let Some(frame) = receiver.recv().await {
            samples.extend_from_slice(&frame.samples);
        }
        samples
    });

    for i in 0..150 {
        let samples = generate_tone(440.0, i);
        let frame = AudioFrame::new(samples, SAMPLE_RATE, 1, (i * FRAME_SIZE) as u32);
        if sender.send(frame).await.is_err() {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }

    println!("[ALICE] Hanging up...");
    handle.hangup().await?;
    alice.wait_for_ended(handle.id()).await?;

    let received = recv_task.await.unwrap_or_default();
    save_wav("alice_received.wav", &received)?;

    bob_task.await.unwrap().unwrap();
    println!("Done.");
    Ok(())
}
