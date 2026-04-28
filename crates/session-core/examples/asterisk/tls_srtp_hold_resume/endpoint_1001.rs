//! Asterisk TLS/SRTP hold/resume endpoint 1001: register over SIP TLS,
//! call 1002 with mandatory SDES-SRTP, hold, resume, and verify audio.

#[path = "../common.rs"]
mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    endpoint_config, init_tracing, load_env, post_register_settle_duration, register_endpoint,
    save_wav, send_tone_segment, ExampleResult,
};
use rvoip_session_core::{SessionHandle, StreamPeer};
use tokio::time::sleep;

const PRE_HOLD_TONE_HZ: f32 = 440.0;
const DURING_HOLD_TONE_HZ: f32 = 550.0;
const POST_RESUME_TONE_HZ: f32 = 660.0;
const TONE_FRAMES_PER_PHASE: usize = 100;
const HOLD_TONE_FRAMES: usize = 50;

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("1001", 5070, 16000, 16100)?;
    let mut peer = StreamPeer::with_config(cfg.tls_srtp_stream_config()?).await?;
    println!(
        "[1001] Security: SIP TLS via sips:/transport=tls; SRTP mandatory (RTP/SAVP + a=crypto)."
    );
    let registration = register_endpoint(&mut peer, &cfg).await?;

    let settle = post_register_settle_duration()?;
    if !settle.is_zero() {
        println!(
            "[1001] Waiting {}s for Asterisk OPTIONS qualify before calling...",
            settle.as_secs()
        );
        sleep(settle).await;
    }

    let target = cfg.outbound_call_uri("1002");
    println!("[1001] Calling {}...", target);
    let handle = peer.call(&target).await?;
    peer.wait_for_answered(handle.id()).await?;
    println!("[1001] Call established over TLS with mandatory SRTP.");

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

    let mut frame_index = 0usize;
    println!("[1001] Sending pre-hold {:.0}Hz tone.", PRE_HOLD_TONE_HZ);
    send_tone_segment(
        &sender,
        PRE_HOLD_TONE_HZ,
        TONE_FRAMES_PER_PHASE,
        &mut frame_index,
    )
    .await?;

    println!("[1001] Putting call on hold...");
    handle.hold().await?;
    wait_for_hold_state(&handle).await?;
    println!("[1001] On hold: {}", handle.is_on_hold().await);

    println!(
        "[1001] Sending best-effort during-hold {:.0}Hz tone.",
        DURING_HOLD_TONE_HZ
    );
    send_tone_segment(
        &sender,
        DURING_HOLD_TONE_HZ,
        HOLD_TONE_FRAMES,
        &mut frame_index,
    )
    .await?;
    sleep(Duration::from_millis(500)).await;

    println!("[1001] Resuming call...");
    handle.resume().await?;
    wait_for_active_state(&handle).await?;
    println!("[1001] Active again: {}", handle.is_active().await);

    println!(
        "[1001] Sending post-resume {:.0}Hz tone.",
        POST_RESUME_TONE_HZ
    );
    send_tone_segment(
        &sender,
        POST_RESUME_TONE_HZ,
        TONE_FRAMES_PER_PHASE,
        &mut frame_index,
    )
    .await?;

    drop(sender);
    println!("[1001] Tone phases complete; hanging up.");
    handle.hangup().await?;
    handle.wait_for_end(Some(Duration::from_secs(8))).await.ok();

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
    let wav = save_wav(
        &cfg.output_dir,
        "tls_srtp_hold_resume_1001_received.wav",
        &received,
    )?;
    println!("[1001] Received audio saved to {}", wav.display());

    peer.unregister(&registration).await.ok();
    println!("[1001] Done.");
    Ok(())
}

async fn wait_for_hold_state(handle: &SessionHandle) -> ExampleResult<()> {
    for _ in 0..30 {
        if handle.is_on_hold().await {
            return Ok(());
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err("call did not reach OnHold within 6s".into())
}

async fn wait_for_active_state(handle: &SessionHandle) -> ExampleResult<()> {
    for _ in 0..30 {
        if handle.is_active().await {
            return Ok(());
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err("call did not return to Active within 6s".into())
}
