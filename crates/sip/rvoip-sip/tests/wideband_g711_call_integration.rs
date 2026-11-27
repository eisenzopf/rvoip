//! End-to-end wideband G.711 call over loopback: two in-process endpoints, no
//! PBX.
//!
//! Wideband G.711 is μ-law/A-law sampled at 16 kHz on a dynamic payload type
//! (`121 PCMU/16000`, `120 PCMA/16000`). The tones prove the rate, not just the codec name:
//! Bob sends 6 kHz, which does not exist at 8 kHz (Nyquist is 4 kHz), so it
//! only arrives if both directions really run at 16 kHz.
//!
//! The second call offers narrowband first and wideband last, and checks that the answer still picks wideband.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::api::unified::Config;
use rvoip_sip::StreamPeer;

const FRAMES: usize = 150; // 3 s at 20 ms per frame
const RATE: u32 = 16_000;
const FRAME_SIZE: usize = 320; // 20 ms at 16 kHz

const ALICE_TONE_HZ: f32 = 1_000.0;
/// Above the 4 kHz limit of narrowband audio.
const BOB_TONE_HZ: f32 = 6_000.0;

const PCMA_WB_PT: u8 = 120;
const PCMU_WB_PT: u8 = 121;
const TELEPHONE_EVENT_PT: u8 = 101;

const MIN_RECEIVED_FRAMES: usize = 50;
const DOMINANCE_RATIO: f32 = 5.0;

#[derive(Clone, Copy)]
struct Ports {
    alice_sip: u16,
    bob_sip: u16,
    alice_media: (u16, u16),
    bob_media: (u16, u16),
}

fn generate_tone(freq: f32, frame_num: usize) -> Vec<i16> {
    (0..FRAME_SIZE)
        .map(|j| {
            let t = (frame_num * FRAME_SIZE + j) as f32 / RATE as f32;
            (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

/// Goertzel magnitude at `target_hz`, as in the AMR and PCMU call tests.
fn goertzel_magnitude(samples: &[i16], sample_rate: f32, target_hz: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let n = samples.len() as f32;
    let k = (0.5 + (n * target_hz) / sample_rate).floor();
    let omega = (2.0 * std::f32::consts::PI * k) / n;
    let coeff = 2.0 * omega.cos();
    let (mut q1, mut q2) = (0.0f32, 0.0f32);
    for &s in samples {
        let q0 = coeff * q1 - q2 + (s as f32);
        q2 = q1;
        q1 = q0;
    }
    (q1 * q1 + q2 * q2 - q1 * q2 * coeff).sqrt()
}

fn config(name: &str, sip_port: u16, media: (u16, u16), codecs: Vec<u8>) -> Config {
    let mut config = Config::local(name, sip_port).with_media_ports(media.0, media.1);
    config.offered_codecs = codecs;
    config
}

/// Place one call from Alice to Bob, stream a tone each way, and check the
/// codec both sides negotiated and the audio both received.
async fn run_call(
    ports: Ports,
    alice_codecs: Vec<u8>,
    bob_codecs: Vec<u8>,
    expected: (&str, u8),
) -> String {
    let _ = tracing_subscriber::fmt::try_init();

    let mut bob =
        StreamPeer::with_config(config("bob", ports.bob_sip, ports.bob_media, bob_codecs))
            .await
            .expect("bob peer starts");
    let mut alice = StreamPeer::with_config(config(
        "alice",
        ports.alice_sip,
        ports.alice_media,
        alice_codecs,
    ))
    .await
    .expect("alice peer starts");

    let bob_received = Arc::new(Mutex::new(Vec::<i16>::new()));
    let bob_buf = bob_received.clone();
    let offer_sdp = Arc::new(Mutex::new(String::new()));
    let offer_slot = offer_sdp.clone();
    let bob_media = Arc::new(Mutex::new(None));
    let bob_media_slot = bob_media.clone();

    let bob_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(10), bob.wait_for_incoming())
            .await
            .expect("bob timed out waiting for the INVITE")
            .expect("bob received an incoming call");
        if let (Some(sdp), Ok(mut slot)) = (incoming.sdp.clone(), offer_slot.lock()) {
            *slot = sdp;
        }
        let handle = incoming.accept().await.expect("bob accepts the call");
        let negotiated = handle.negotiated_media().await.ok().flatten();
        if let Ok(mut slot) = bob_media_slot.lock() {
            *slot = negotiated;
        }

        let audio = handle.audio().await.expect("bob opens the audio stream");
        let (sender, mut receiver) = audio.split();
        let recv = tokio::spawn(async move {
            while let Some(frame) = receiver.recv().await {
                assert_eq!(frame.sample_rate, RATE, "bob received a non-16 kHz frame");
                if let Ok(mut buf) = bob_buf.lock() {
                    buf.extend_from_slice(&frame.samples);
                }
            }
        });

        for i in 0..FRAMES {
            let frame = AudioFrame::new(
                generate_tone(BOB_TONE_HZ, i),
                RATE,
                1,
                (i * FRAME_SIZE) as u32,
            );
            if sender.send(frame).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(sender);
        let _ = handle.wait_for_end(Some(Duration::from_secs(5))).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        recv.abort();
    });

    let call_id = alice
        .invite(format!("sip:bob@127.0.0.1:{}", ports.bob_sip))
        .send()
        .await
        .expect("alice sends the INVITE");
    let handle = tokio::time::timeout(Duration::from_secs(10), alice.wait_for_answered(&call_id))
        .await
        .expect("alice timed out waiting for the answer")
        .expect("alice's call was answered");

    let alice_media = handle
        .negotiated_media()
        .await
        .expect("negotiated media")
        .expect("alice negotiated media");
    assert_eq!(alice_media.codec, expected.0, "alice: {alice_media:?}");
    assert_eq!(
        alice_media.payload_type, expected.1,
        "alice: {alice_media:?}"
    );
    assert_eq!(alice_media.clock_rate, RATE, "alice: {alice_media:?}");

    let audio = handle.audio().await.expect("alice opens the audio stream");
    let (sender, mut receiver) = audio.split();
    let alice_received = Arc::new(Mutex::new(Vec::<i16>::new()));
    let alice_buf = alice_received.clone();
    let alice_recv = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            assert_eq!(frame.sample_rate, RATE, "alice received a non-16 kHz frame");
            if let Ok(mut buf) = alice_buf.lock() {
                buf.extend_from_slice(&frame.samples);
            }
        }
    });

    for i in 0..FRAMES {
        let frame = AudioFrame::new(
            generate_tone(ALICE_TONE_HZ, i),
            RATE,
            1,
            (i * FRAME_SIZE) as u32,
        );
        if sender.send(frame).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(sender);

    handle.hangup().await.expect("alice hangs up");
    let _ = tokio::time::timeout(Duration::from_secs(5), alice.wait_for_ended(&call_id)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    alice_recv.abort();
    tokio::time::timeout(Duration::from_secs(10), bob_task)
        .await
        .expect("bob finishes")
        .expect("bob's task did not panic");

    let bob_media = bob_media
        .lock()
        .unwrap()
        .clone()
        .expect("bob negotiated media");
    assert_eq!(bob_media.codec, expected.0, "bob: {bob_media:?}");
    assert_eq!(bob_media.clock_rate, RATE, "bob: {bob_media:?}");

    let alice_samples = alice_received.lock().map(|g| g.clone()).unwrap_or_default();
    let bob_samples = bob_received.lock().map(|g| g.clone()).unwrap_or_default();
    let min_samples = MIN_RECEIVED_FRAMES * FRAME_SIZE;
    assert!(
        alice_samples.len() >= min_samples,
        "alice received {} samples (expected >= {min_samples})",
        alice_samples.len()
    );
    assert!(
        bob_samples.len() >= min_samples,
        "bob received {} samples (expected >= {min_samples})",
        bob_samples.len()
    );

    let rate = RATE as f32;
    let alice_peer = goertzel_magnitude(&alice_samples, rate, BOB_TONE_HZ);
    let alice_self = goertzel_magnitude(&alice_samples, rate, ALICE_TONE_HZ);
    let bob_peer = goertzel_magnitude(&bob_samples, rate, ALICE_TONE_HZ);
    let bob_self = goertzel_magnitude(&bob_samples, rate, BOB_TONE_HZ);
    let ratio = |peer: f32, own: f32| if own > 1.0 { peer / own } else { f32::INFINITY };
    eprintln!(
        "alice 6kHz {alice_peer:.1} / 1kHz {alice_self:.1}; bob 1kHz {bob_peer:.1} / 6kHz {bob_self:.1}"
    );
    assert!(
        ratio(alice_peer, alice_self) >= DOMINANCE_RATIO,
        "alice did not hear bob's 6 kHz tone: {alice_peer:.1} vs {alice_self:.1}"
    );
    assert!(
        ratio(bob_peer, bob_self) >= DOMINANCE_RATIO,
        "bob did not hear alice's 1 kHz tone: {bob_peer:.1} vs {bob_self:.1}"
    );

    offer_sdp.lock().map(|g| g.clone()).unwrap_or_default()
}

/// Wideband μ-law only: the offer carries it on 121, and a 6 kHz tone
/// crosses the call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wideband_pcmu_call_carries_audio_above_4_khz() {
    let offer = run_call(
        Ports {
            alice_sip: 42_400,
            bob_sip: 42_401,
            alice_media: (42_500, 42_550),
            bob_media: (42_560, 42_610),
        },
        vec![PCMU_WB_PT, TELEPHONE_EVENT_PT],
        vec![PCMU_WB_PT, TELEPHONE_EVENT_PT],
        ("PCMU", PCMU_WB_PT),
    )
    .await;
    assert!(offer.contains("a=rtpmap:121 PCMU/16000"), "{offer}");
    assert!(!offer.contains("PCMU/8000"), "{offer}");
}

/// Narrowband first, `0 8 120 121 101`: an answerer
/// with wideband enabled still picks wideband, the first offered (A-law).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn answerer_prefers_wideband_g711_in_a_narrowband_first_offer() {
    let offer = run_call(
        Ports {
            alice_sip: 42_402,
            bob_sip: 42_403,
            alice_media: (42_620, 42_670),
            bob_media: (42_680, 42_730),
        },
        vec![0, 8, PCMA_WB_PT, PCMU_WB_PT, TELEPHONE_EVENT_PT],
        vec![PCMU_WB_PT, PCMA_WB_PT, 0, 8, TELEPHONE_EVENT_PT],
        ("PCMA", PCMA_WB_PT),
    )
    .await;
    assert!(offer.contains("a=rtpmap:120 PCMA/16000"), "{offer}");
    assert!(offer.contains("a=rtpmap:121 PCMU/16000"), "{offer}");
}
