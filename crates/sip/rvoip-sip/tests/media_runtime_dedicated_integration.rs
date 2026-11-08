//! A real call whose media runs on dedicated (current-thread) media lanes.
//!
//! The media executor is process-wide and configured once, so this binary
//! installs one layout: two dedicated lanes, each a single thread. Two
//! in-process endpoints place a call over loopback and stream PCMU both ways;
//! both RTP transports must land on the dedicated lanes and audio must flow
//! in both directions.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_rtp_core::task_runtime::{media_runtime_snapshot, MediaLaneSnapshot};
use rvoip_sip::api::unified::Config;
use rvoip_sip::{MediaLanes, MediaRuntimeConfig, SessionHandle, StreamPeer};

const SAMPLE_RATE: u32 = 8_000;
const FRAME_SAMPLES: usize = 160;
const FRAMES: usize = 100;

fn dedicated_lanes() -> MediaRuntimeConfig {
    MediaRuntimeConfig::new(MediaLanes::dedicated(2))
}

fn config(name: &str, sip_port: u16, media: (u16, u16)) -> Config {
    Config::local(name, sip_port)
        .with_media_ports(media.0, media.1)
        .with_media_runtime(dedicated_lanes())
}

fn tone(freq: f32, frame: usize) -> Vec<i16> {
    (0..FRAME_SAMPLES)
        .map(|j| {
            let t = (frame * FRAME_SAMPLES + j) as f32 / SAMPLE_RATE as f32;
            (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

/// Stream a tone for `FRAMES` 20 ms frames and count the frames received
/// meanwhile.
async fn stream_audio(handle: SessionHandle, freq: f32) -> usize {
    let audio = handle.audio().await.expect("audio stream opens");
    let (sender, mut receiver) = audio.split();
    let received = Arc::new(AtomicUsize::new(0));
    let counter = received.clone();
    let drain = tokio::spawn(async move {
        while receiver.recv().await.is_some() {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    });
    for i in 0..FRAMES {
        let frame = AudioFrame::new(tone(freq, i), SAMPLE_RATE, 1, (i * FRAME_SAMPLES) as u32);
        if sender.send(frame).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Let the last packets arrive.
    tokio::time::sleep(Duration::from_millis(200)).await;
    drain.abort();
    received.load(Ordering::Relaxed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_media_flows_both_ways_on_dedicated_lanes() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_sip, bob_sip) = (37_610, 37_611);
    let bob_config = config("bob", bob_sip, (37_650, 37_669));
    bob_config.validate().expect("a dedicated layout is valid");

    let mut bob = StreamPeer::with_config(bob_config)
        .await
        .expect("bob starts");
    let mut alice = StreamPeer::with_config(config("alice", alice_sip, (37_620, 37_639)))
        .await
        .expect("alice starts with the same layout");

    let bob_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(10), bob.wait_for_incoming())
            .await
            .expect("bob timed out waiting for the INVITE")
            .expect("bob received the INVITE");
        let handle = incoming.accept().await.expect("bob accepts");
        (bob, handle)
    });
    let call_id = alice
        .invite(format!("sip:bob@127.0.0.1:{bob_sip}"))
        .send()
        .await
        .expect("alice sends the INVITE");
    let alice_handle =
        tokio::time::timeout(Duration::from_secs(10), alice.wait_for_answered(&call_id))
            .await
            .expect("alice timed out waiting for the answer")
            .expect("call answered");
    let (bob, bob_handle) = bob_task.await.unwrap();

    let alice_audio = tokio::spawn(stream_audio(alice_handle.clone(), 440.0));
    let bob_audio = tokio::spawn(stream_audio(bob_handle.clone(), 880.0));
    let (alice_received, bob_received) = (alice_audio.await.unwrap(), bob_audio.await.unwrap());
    assert!(
        alice_received >= FRAMES / 2 && bob_received >= FRAMES / 2,
        "audio must flow both ways: alice received {alice_received}, bob {bob_received} of {FRAMES}"
    );

    let snapshot = media_runtime_snapshot().expect("the call started the media executor");
    assert_eq!(snapshot.config, dedicated_lanes());
    assert_eq!(snapshot.lanes.len(), 2);
    assert!(snapshot
        .lanes
        .iter()
        .all(|lane: &MediaLaneSnapshot| lane.worker_threads == 1 && lane.pinned_cpu.is_none()));
    // Each endpoint's RTP transport was placed on a lane, round robin.
    let transports: Vec<u64> = snapshot
        .lanes
        .iter()
        .map(|lane| lane.transports_placed)
        .collect();
    assert!(
        transports.iter().sum::<u64>() >= 2 && transports.iter().all(|placed| *placed >= 1),
        "transports per lane: {transports:?}"
    );

    alice_handle.hangup().await.ok();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop((alice, bob, bob_handle));
}
