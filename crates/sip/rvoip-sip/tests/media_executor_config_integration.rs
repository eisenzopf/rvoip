//! `Config::media_executor` selects the process-wide media executor, and a
//! real call runs on executor v2.
//!
//! The executor is global and configured once, so this binary installs
//! executor v2 (with two dedicated lanes) and checks every coordinator
//! against it. Two in-process endpoints place a call over loopback and stream
//! PCMU both ways: both RTP transports must receive on their lane pollers,
//! account RTP inline on the socket owner, and audio must flow in both
//! directions, paced at 20 ms although both lanes run on v2's 2 ms lane tick
//! with the lane timing wheel and the pass clock. SRTP calls, with RTCP
//! multiplexed and with a separate RTCP port, run on the same executor: the
//! receive loop owns the inbound SRTP context only when RTCP shares the RTP
//! socket.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_media_core::types::AudioFrame;
use rvoip_rtp_core::executor::{
    inline_executor_counters, lane_wheel_counters, media_executor_config,
};
use rvoip_rtp_core::lane_tick::lane_tick_counters;
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

/// Stream a tone for `FRAMES` 20 ms frames and return the frames received
/// meanwhile: (RTP timestamp, when it arrived).
async fn stream_audio(handle: SessionHandle, freq: f32) -> Vec<(u32, Instant)> {
    let audio = handle.audio().await.expect("audio stream opens");
    let (sender, mut receiver) = audio.split();
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = received.clone();
    let drain = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            log.lock().unwrap().push((frame.timestamp, Instant::now()));
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
    let frames = std::mem::take(&mut *received.lock().unwrap());
    frames
}

/// The received frames are consecutive 20 ms frames (RTP timestamps 160
/// apart) delivered at a 20 ms cadence: the median gap between deliveries
/// is within a lane tick quantum (plus scheduling noise) of 20 ms, and the
/// stream spans its frame count in time.
fn assert_paced_at_20ms(who: &str, frames: &[(u32, Instant)]) {
    let steps: Vec<u32> = frames
        .windows(2)
        .map(|pair| pair[1].0.wrapping_sub(pair[0].0))
        .collect();
    let in_step = steps.iter().filter(|step| **step == 160).count();
    assert!(
        in_step * 10 >= steps.len() * 9,
        "{who}: {in_step} of {} timestamp steps are one 20 ms frame",
        steps.len()
    );
    let mut gaps: Vec<Duration> = frames
        .windows(2)
        .map(|pair| pair[1].1.saturating_duration_since(pair[0].1))
        .collect();
    gaps.sort();
    let median = gaps[gaps.len() / 2];
    assert!(
        median >= Duration::from_millis(16) && median <= Duration::from_millis(24),
        "{who}: median delivery gap {median:?}"
    );
    let span = frames.last().unwrap().1 - frames[0].1;
    let expected = Duration::from_millis(20) * (frames.len() as u32 - 1);
    assert!(
        span >= expected * 8 / 10 && span <= expected * 12 / 10 + Duration::from_millis(100),
        "{who}: {} frames spanned {span:?}",
        frames.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_media_flows_both_ways_on_executor_v2() {
    let _ = tracing_subscriber::fmt::try_init();
    let tick = MediaExecutorConfig::v2().lane_tick.expect("v2 ticks");
    assert_eq!(tick.quantum, Duration::from_millis(2));
    assert!(tick.grid_timers);
    let v2 = MediaExecutorConfig::v2();
    assert!(v2.lane_timer_wheel_active() && v2.lane_pass_clock_active());
    let before = inline_executor_counters();
    let wheel_before = lane_wheel_counters();
    let tick_before = lane_tick_counters();
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
    let (alice_frames, bob_frames) = (alice_audio.await.unwrap(), bob_audio.await.unwrap());
    let (alice_received, bob_received) = (alice_frames.len(), bob_frames.len());
    assert!(
        alice_received >= FRAMES / 2 && bob_received >= FRAMES / 2,
        "audio must flow both ways: alice received {alice_received}, bob {bob_received} of {FRAMES}"
    );
    assert_paced_at_20ms("alice", &alice_frames);
    assert_paced_at_20ms("bob", &bob_frames);

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
    // The lanes are dedicated current-thread lanes, so their pollers ran on
    // the tick: they slept between ticks instead of waking per packet.
    let tick_after = lane_tick_counters();
    assert!(
        tick_after.tick_lanes >= 1,
        "lane pollers tick on dedicated lanes"
    );
    assert_eq!(
        tick_after.unsupported_lanes, tick_before.unsupported_lanes,
        "every lane could tick"
    );
    assert!(tick_after.tick_passes > tick_before.tick_passes);
    assert!(tick_after.tick_sleeps > tick_before.tick_sleeps);
    // Each lane pass served one clock reading to the loops it polled (the
    // timing wheel's equivalence with the heap is proven in rtp-core; an
    // in-order call files few lane-clock deadlines).
    let wheel_after = lane_wheel_counters();
    assert!(
        wheel_after.pass_clock_fresh_reads > wheel_before.pass_clock_fresh_reads,
        "pass clock readings: {wheel_before:?} -> {wheel_after:?}"
    );
    assert!(
        wheel_after.pass_clock_cached_reads > wheel_before.pass_clock_cached_reads,
        "reads served from a pass reading: {wheel_before:?} -> {wheel_after:?}"
    );

    // Each session's periodic RTCP task runs on its socket's lane poller,
    // and its socket owner publishes the receive statistics.
    assert!(after.rtcp_lane_tasks >= before.rtcp_lane_tasks + 2);
    assert!(after.owner_receive_stats >= before.owner_receive_stats + 2);

    alice_handle.hangup().await.ok();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop((alice, bob, bob_handle));
}

/// Place an SRTP call from `alice` to `bob` and stream a tone both ways;
/// returns the frames each side received.
async fn srtp_call(alice: Config, bob: Config, rtcp_mux: bool) -> (usize, usize) {
    secure_call(alice, bob, rtcp_mux, |_| {}, false).await
}

/// [`srtp_call`] with extra per-endpoint configuration (the keying mode);
/// with `handshake`, media starts once both ends installed their contexts.
async fn secure_call(
    mut alice: Config,
    mut bob: Config,
    rtcp_mux: bool,
    configure: impl Fn(&mut Config),
    handshake: bool,
) -> (usize, usize) {
    for config in [&mut alice, &mut bob] {
        config.offer_srtp = true;
        config.srtp_required = true;
        configure(config);
        if !rtcp_mux {
            config.offer_rtcp_mux = false;
            config.rtcp_mux_required = false;
            config.rtcp_non_mux = true;
        }
    }
    let bob_sip = bob.sip_port;
    let mut bob = StreamPeer::with_config(bob).await.expect("bob starts");
    let mut alice = StreamPeer::with_config(alice).await.expect("alice starts");
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
    if handshake {
        for handle in [&alice_handle, &bob_handle] {
            handle
                .wait_for_media_security(Some(Duration::from_secs(12)))
                .await
                .expect("SRTP contexts installed from the handshake");
        }
    }
    let alice_audio = tokio::spawn(stream_audio(alice_handle.clone(), 440.0));
    let bob_audio = tokio::spawn(stream_audio(bob_handle.clone(), 880.0));
    let received = (
        alice_audio.await.unwrap().len(),
        bob_audio.await.unwrap().len(),
    );
    alice_handle.hangup().await.ok();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop((alice, bob, bob_handle));
    received
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn srtp_calls_with_and_without_rtcp_mux_flow_both_ways_on_executor_v2() {
    let _ = tracing_subscriber::fmt::try_init();
    let before = inline_executor_counters();
    let (alice_received, bob_received) = srtp_call(
        config("alice-srtp", 37_801, (37_810, 37_829)),
        config("bob-srtp", 37_802, (37_830, 37_849)),
        true,
    )
    .await;
    assert!(
        alice_received >= FRAMES / 2 && bob_received >= FRAMES / 2,
        "SRTP audio with rtcp-mux: alice received {alice_received}, bob {bob_received} of {FRAMES}"
    );
    let after = inline_executor_counters();
    assert!(
        after.owner_srtp_loops >= before.owner_srtp_loops + 2,
        "both receive loops own their SRTP context: {} -> {}",
        before.owner_srtp_loops,
        after.owner_srtp_loops
    );
    assert!(after.srtp_locks_avoided > before.srtp_locks_avoided);

    // A separate RTCP port: the RTCP socket's receive task shares the SRTP
    // context, which stays behind its lock; media still flows both ways.
    let (alice_received, bob_received) = srtp_call(
        config("alice-srtp-non-mux", 37_803, (37_850, 37_869)),
        config("bob-srtp-non-mux", 37_804, (37_870, 37_889)),
        false,
    )
    .await;
    assert!(
        alice_received >= FRAMES / 2 && bob_received >= FRAMES / 2,
        "SRTP audio without rtcp-mux: alice received {alice_received}, bob {bob_received} of {FRAMES}"
    );
}

/// DTLS-SRTP: the keys arrive from the handshake on the RTP socket after the
/// receive loop started; the owning loop adopts the installed context.
#[cfg(feature = "dtls-srtp")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dtls_srtp_call_flows_both_ways_on_executor_v2() {
    use rvoip_sip::api::unified::SrtpKeyingMode;

    let _ = tracing_subscriber::fmt::try_init();
    let before = inline_executor_counters();
    let (alice_received, bob_received) = secure_call(
        config("alice-dtls", 37_805, (37_890, 37_909)),
        config("bob-dtls", 37_806, (37_910, 37_929)),
        true,
        |config| config.srtp_keying = SrtpKeyingMode::DtlsSrtp,
        true,
    )
    .await;
    assert!(
        alice_received >= FRAMES / 2 && bob_received >= FRAMES / 2,
        "DTLS-SRTP audio: alice received {alice_received}, bob {bob_received} of {FRAMES}"
    );
    let after = inline_executor_counters();
    assert!(after.owner_srtp_loops >= before.owner_srtp_loops + 2);
    assert!(after.owner_srtp_handoffs >= before.owner_srtp_handoffs + 2);
}
