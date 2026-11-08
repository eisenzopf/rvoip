//! `Config::media_executor` selects the process-wide media executor, and a
//! real call runs on executor v2.
//!
//! The executor is global and configured once, so this binary installs
//! executor v2 (with two dedicated lanes) and checks every coordinator
//! against it. Two in-process endpoints place a call over loopback and stream
//! PCMU both ways: both RTP transports must receive on their lane pollers,
//! account RTP inline on the socket owner, and audio must flow in both
//! directions.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_rtp_core::executor::{inline_executor_counters, media_executor_config};
use rvoip_sip::api::unified::Config;
use rvoip_sip::{
    MediaExecutorConfig, MediaExecutorMode, MediaLanes, MediaRuntimeConfig, SessionError,
    SessionHandle, StreamPeer, UnifiedCoordinator,
};

const SAMPLE_RATE: u32 = 8_000;
const FRAME_SAMPLES: usize = 160;
const FRAMES: usize = 100;

fn lanes() -> MediaRuntimeConfig {
    MediaRuntimeConfig::new(MediaLanes::dedicated(2))
}

fn config(name: &str, sip_port: u16, media: (u16, u16)) -> Config {
    Config::local(name, sip_port)
        .with_media_ports(media.0, media.1)
        .with_media_runtime(lanes())
        .with_media_executor(MediaExecutorConfig::v2())
}

fn config_error_detail(error: &SessionError) -> &str {
    let SessionError::ConfigError(detail) = error else {
        panic!("expected typed ConfigError, got {error:?}");
    };
    detail
}

#[test]
fn media_executor_defaults_to_v1() {
    assert_eq!(Config::default().media_executor, None);
    assert_eq!(Config::local("executor", 5060).media_executor, None);
    assert_eq!(MediaExecutorConfig::default().mode, MediaExecutorMode::V1);
    let config = Config::local("executor", 5060).with_media_executor(MediaExecutorConfig::v2());
    assert_eq!(config.media_executor, Some(MediaExecutorConfig::v2()));
    config.validate().expect("v2 is valid");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coordinator_installs_the_executor_and_rejects_a_conflicting_one() {
    let first = UnifiedCoordinator::new(config("executor-first", 0, (37_780, 37_789)))
        .await
        .expect("the first coordinator installs executor v2");
    let unset = UnifiedCoordinator::new(Config::local("executor-unset", 0))
        .await
        .expect("a coordinator without an executor uses the installed one");
    assert_eq!(media_executor_config(), MediaExecutorConfig::v2());

    let error = match UnifiedCoordinator::new(
        Config::local("executor-conflict", 0).with_media_executor(MediaExecutorConfig::v1()),
    )
    .await
    {
        Ok(_) => panic!("a conflicting executor must not start"),
        Err(error) => error,
    };
    let detail = config_error_detail(&error);
    assert!(
        detail.contains("media_executor") && detail.contains("already configured"),
        "unexpected error: {error}"
    );

    for coordinator in [first, unset] {
        coordinator
            .shutdown_gracefully(Some(Duration::ZERO))
            .await
            .expect("shutdown");
    }
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
async fn call_media_flows_both_ways_on_executor_v2() {
    let _ = tracing_subscriber::fmt::try_init();
    let before = inline_executor_counters();
    let (alice_sip, bob_sip) = (37_710, 37_711);
    let mut bob = StreamPeer::with_config(config("bob", bob_sip, (37_750, 37_769)))
        .await
        .expect("bob starts");
    let mut alice = StreamPeer::with_config(config("alice", alice_sip, (37_720, 37_739)))
        .await
        .expect("alice starts with the same executor");

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

    // Both endpoints' receive loops ran on their lane pollers and received
    // the call's RTP there.
    let after = inline_executor_counters();
    assert!(
        after.lane_sockets >= before.lane_sockets + 2,
        "receive loops hosted on lane pollers: {} -> {}",
        before.lane_sockets,
        after.lane_sockets
    );
    assert!(after.lane_wakeups > before.lane_wakeups);
    assert!(
        after.lane_datagrams >= before.lane_datagrams + FRAMES as u64,
        "datagrams received on lane pollers: {} -> {}",
        before.lane_datagrams,
        after.lane_datagrams
    );

    alice_handle.hangup().await.ok();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop((alice, bob, bob_handle));
}
