//! Audio caller (Alice) — calls Bob, sends 440Hz tone, saves received audio.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_audio_alice
//! Or with bob:     ./examples/streampeer/audio/run.sh

use rvoip_media_core::types::AudioFrame;
use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

const SAMPLE_RATE: u32 = 8000;
const FRAME_SIZE: usize = 160; // 20ms at 8kHz

fn env_u16(k: &str, default: u16) -> u16 {
    std::env::var(k)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_string(k: &str, default: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| default.to_string())
}

fn generate_tone(freq: f32, frame_num: usize) -> Vec<i16> {
    (0..FRAME_SIZE)
        .map(|j| {
            let t = (frame_num * FRAME_SIZE + j) as f32 / SAMPLE_RATE as f32;
            (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

fn save_wav(
    out_dir: &str,
    name: &str,
    samples: &[i16],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(out_dir)?;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let path = format!("{}/{}", out_dir, name);
    let mut writer = hound::WavWriter::create(&path, spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    println!("Saved {}", path);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let alice_port = env_u16("ALICE_SIP_PORT", 5060);
    let bob_port = env_u16("BOB_SIP_PORT", 5061);
    let media_start = env_u16("ALICE_MEDIA_PORT_START", 10000);
    let media_end = env_u16("ALICE_MEDIA_PORT_END", 10100);
    let out_dir = env_string("AUDIO_OUTPUT_DIR", "output");

    let mut alice = StreamPeer::with_config(Config {
        media_port_start: media_start,
        media_port_end: media_end,
        ..Config::local("alice", alice_port)
    })
    .await?;

    println!("[ALICE] Calling Bob on port {}...", bob_port);
    let handle = alice
        .call(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .await?;
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

    // Drop sender so recv_task's channel closes
    drop(sender);

    println!("[ALICE] Hanging up...");
    handle.hangup().await?;
    alice.wait_for_ended(handle.id()).await?;

    let received = recv_task.await.unwrap_or_default();
    save_wav(&out_dir, "alice_received.wav", &received)?;
    println!("[ALICE] Done.");

    std::process::exit(0);
}
