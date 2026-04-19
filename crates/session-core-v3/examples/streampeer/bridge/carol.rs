//! Bridge example — Carol (callee) accepts the bridged call, sends 880 Hz,
//! records what comes back. A working bridge means Alice's 440 Hz tone
//! lands in `carol_received.wav`.
//!
//! Run standalone:  cargo run -p rvoip-session-core-v3 --example streampeer_bridge_carol
//! Or with bridge + alice: ./examples/streampeer/bridge/run.sh

use rvoip_media_core::types::AudioFrame;
use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

const SAMPLE_RATE: u32 = 8000;
const FRAME_SIZE: usize = 160;

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
    println!("[CAROL] Saved {}", path);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let carol_port = env_u16("CAROL_SIP_PORT", 35591);
    let media_start = env_u16("CAROL_MEDIA_PORT_START", 35820);
    let media_end = env_u16("CAROL_MEDIA_PORT_END", 35870);
    let out_dir = env_string("AUDIO_OUTPUT_DIR", "output");

    let mut carol = StreamPeer::with_config(Config {
        media_port_start: media_start,
        media_port_end: media_end,
        ..Config::local("carol", carol_port)
    })
    .await?;

    println!("[CAROL] Waiting for bridged call...");
    let incoming = carol.wait_for_incoming().await?;
    println!("[CAROL] Call from {}", incoming.from);
    let handle = incoming.accept().await?;

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
        let samples = generate_tone(880.0, i);
        let frame = AudioFrame::new(samples, SAMPLE_RATE, 1, (i * FRAME_SIZE) as u32);
        if sender.send(frame).await.is_err() {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }

    drop(sender);
    handle.wait_for_end(Some(Duration::from_secs(5))).await.ok();

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

    save_wav(&out_dir, "carol_received.wav", &received)?;
    println!("[CAROL] Done ({} samples received).", received.len());
    std::process::exit(0);
}
