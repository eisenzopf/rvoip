//! Asterisk UDP hold/resume endpoint 2002: register, answer 2001, send a steady
//! reference tone, and record everything received from the caller.

#[path = "../common.rs"]
mod common;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use common::{
    endpoint_config, expect_remote_hold_events, generate_tone, init_tracing, load_env,
    register_endpoint, save_wav, wait_for_remote_hold_on_events, wait_for_remote_resume_on_events,
    ExampleResult, ENDPOINT_2002_TONE_HZ, FRAME_SIZE, SAMPLE_RATE,
};
use rvoip_media_core::types::AudioFrame;
use rvoip_session_core::StreamPeer;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("2002", 5082, 17120, 17220)?;
    let mut peer = StreamPeer::with_config(cfg.stream_config()).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;
    println!("[2002] Registered; waiting for call.");

    let incoming = peer.wait_for_incoming().await?;
    println!("[2002] Incoming call from {}", incoming.from);
    let handle = incoming.accept().await?;
    println!("[2002] Call answered.");
    let mut call_events = handle.events().await?;

    let audio = handle.audio().await?;
    let (sender, mut receiver) = audio.split();

    let received_buf = Arc::new(Mutex::new(Vec::<i16>::new()));
    let recv_buf = received_buf.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            if let Ok(mut buf) = recv_buf.lock() {
                buf.extend_from_slice(&frame.samples);
            }
        }
    });

    let running = Arc::new(AtomicBool::new(true));
    let send_running = running.clone();
    let send_task = tokio::spawn(async move {
        let mut frame_index = 0usize;
        while send_running.load(Ordering::Relaxed) && sender.is_open() {
            let frame = AudioFrame::new(
                generate_tone(ENDPOINT_2002_TONE_HZ, frame_index),
                SAMPLE_RATE,
                1,
                (frame_index * FRAME_SIZE) as u32,
            );
            if sender.send(frame).await.is_err() {
                break;
            }
            frame_index += 1;
            sleep(Duration::from_millis(20)).await;
        }
    });

    if expect_remote_hold_events()? {
        println!("[2002] Waiting for caller hold indication.");
        wait_for_remote_hold_on_events(&mut call_events, Duration::from_secs(15)).await?;
        println!("[2002] Waiting for caller resume indication.");
        wait_for_remote_resume_on_events(&mut call_events, Duration::from_secs(15)).await?;
    } else {
        println!(
            "[2002] Remote hold/resume event assertion disabled; set ASTERISK_EXPECT_REMOTE_HOLD_EVENTS=1 for PBX profiles that forward hold re-INVITEs."
        );
    }

    let reason = handle.wait_for_end(Some(Duration::from_secs(30))).await?;
    println!("[2002] Call ended: {}", reason);
    running.store(false, Ordering::Relaxed);

    let _ = tokio::time::timeout(Duration::from_secs(2), send_task).await;
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
    let wav = save_wav(&cfg.output_dir, "hold_resume_2002_received.wav", &received)?;
    println!("[2002] Received audio saved to {}", wav.display());

    peer.unregister(&registration).await.ok();
    println!("[2002] Done.");
    Ok(())
}
