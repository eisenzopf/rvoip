//! Audio receiver (Bob) — waits for incoming call, sends 880Hz tone, saves received audio.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_audio_bob
//! Or with alice:   ./examples/streampeer/audio/run.sh

use rvoip_media_core::types::AudioFrame;
use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

const SAMPLE_RATE: u32 = 8000;
const FRAME_SIZE: usize = 160; // 20ms at 8kHz

fn env_u16(k: &str, default: u16) -> u16 {
    std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
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

fn save_wav(out_dir: &str, name: &str, samples: &[i16]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let bob_port = env_u16("BOB_SIP_PORT", 5061);
    let media_start = env_u16("BOB_MEDIA_PORT_START", 10100);
    let media_end = env_u16("BOB_MEDIA_PORT_END", 10200);
    let out_dir = env_string("AUDIO_OUTPUT_DIR", "output");

    let mut bob = StreamPeer::with_config(Config {
        media_port_start: media_start,
        media_port_end: media_end,
        ..Config::local("bob", bob_port)
    })
    .await?;

    println!("[BOB] Waiting for call...");
    let incoming = bob.wait_for_incoming().await?;
    println!("[BOB] Call from {}", incoming.from);
    let handle = incoming.accept().await?;

    // Send 880Hz tone and receive audio. The recv task writes samples
    // into a shared buffer so we can grab whatever arrived even if the
    // underlying channel never formally closes (the UAS-side RTP
    // receiver isn't always dropped promptly after a peer-initiated BYE).
    let audio = handle.audio().await?;
    let (sender, mut receiver) = audio.split();

    let received_buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<i16>::new()));
    let recv_buf = received_buf.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            if let Ok(mut buf) = recv_buf.lock() {
                buf.extend_from_slice(&frame.samples);
            }
        }
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

    // Drop sender so our TX loop stops retaining the audio stream.
    drop(sender);
    handle.wait_for_end(Some(Duration::from_secs(5))).await.ok();

    // Give the RTP pipeline a short drain window then abort the recv
    // task whether its channel closed or not, and pull whatever
    // samples have accumulated so far.
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if recv_task.is_finished() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    recv_task.abort();
    let received = received_buf.lock().map(|g| g.clone()).unwrap_or_default();

    save_wav(&out_dir, "bob_received.wav", &received)?;
    println!("[BOB] Done.");

    std::process::exit(0);
}
