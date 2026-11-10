//! An application that opens `SessionHandle::audio()` the moment a call is
//! answered must get media flowing, whatever keys the call's SRTP.
//!
//! With DTLS-SRTP the keys come from a handshake that finishes after the
//! answer, so the first frames the application sends arrive before the call
//! can protect them. Those frames are dropped; the send pump must keep
//! running and media must flow once the keys are installed. The same test
//! runs with SDES-SRTP and plain RTP, where keys exist at the answer, to
//! show nothing changed for them.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::api::unified::Config;
#[cfg(feature = "dtls-srtp")]
use rvoip_sip::api::unified::SrtpKeyingMode;
use rvoip_sip::{MediaQualityStats, SessionHandle, StreamPeer};

const SAMPLE_RATE: u32 = 8_000;
const FRAME_SAMPLES: usize = 160;

#[derive(Clone, Copy, Debug)]
enum Keying {
    Plain,
    Sdes,
    #[cfg(feature = "dtls-srtp")]
    Dtls,
}

fn config(name: &str, sip_port: u16, media: (u16, u16), keying: Keying) -> Config {
    let mut config = Config::local(name, sip_port).with_media_ports(media.0, media.1);
    match keying {
        Keying::Plain => {}
        Keying::Sdes => {
            config.offer_srtp = true;
            config.srtp_required = true;
        }
        #[cfg(feature = "dtls-srtp")]
        Keying::Dtls => {
            config.offer_srtp = true;
            config.srtp_required = true;
            config.srtp_keying = SrtpKeyingMode::DtlsSrtp;
        }
    }
    config
}

/// Stream a tone from the moment audio opens, counting sends the
/// application saw fail. Every send must succeed: frames the call cannot
/// protect yet are dropped, not reported as a dead stream.
async fn stream_audio(handle: SessionHandle, freq: f32, frames: usize, failures: Arc<AtomicU64>) {
    let audio = handle.audio().await.expect("audio stream opens");
    let (sender, mut receiver) = audio.split();
    let drain = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    for i in 0..frames {
        let samples = (0..FRAME_SAMPLES)
            .map(|j| {
                let t = (i * FRAME_SAMPLES + j) as f32 / SAMPLE_RATE as f32;
                (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
            })
            .collect();
        let frame = AudioFrame::new(samples, SAMPLE_RATE, 1, (i * FRAME_SAMPLES) as u32);
        if let Err(error) = sender.send(frame).await {
            eprintln!("send {i} failed: {error}");
            failures.fetch_add(1, Ordering::Relaxed);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drain.abort();
}

async fn wait_for_media(handle: &SessionHandle, within: Duration) -> MediaQualityStats {
    let deadline = tokio::time::Instant::now() + within;
    let mut last = None;
    while tokio::time::Instant::now() < deadline {
        if let Some(quality) = handle.media_quality().await {
            if quality.packets_sent >= 50 && quality.packets_received >= 50 {
                return quality;
            }
            last = Some(quality);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("media did not flow both ways within {within:?}; last sample: {last:?}");
}

async fn audio_opened_on_answer_flows(
    keying: Keying,
    sip: (u16, u16),
    alice_media: (u16, u16),
    bob_media: (u16, u16),
) {
    let _ = tracing_subscriber::fmt::try_init();
    let mut bob = StreamPeer::with_config(config("bob", sip.1, bob_media, keying))
        .await
        .expect("bob starts");
    let mut alice = StreamPeer::with_config(config("alice", sip.0, alice_media, keying))
        .await
        .expect("alice starts");
    let failures = Arc::new(AtomicU64::new(0));

    // Bob opens audio straight from accept(), before any media keys exist.
    let bob_failures = Arc::clone(&failures);
    let bob_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(10), bob.wait_for_incoming())
            .await
            .expect("bob timed out waiting for the INVITE")
            .expect("bob received the INVITE");
        let handle = incoming.accept().await.expect("bob accepts");
        let audio = tokio::spawn(stream_audio(handle.clone(), 880.0, 300, bob_failures));
        (bob, handle, audio)
    });
    let call_id = alice
        .invite(format!("sip:bob@127.0.0.1:{}", sip.1))
        .send()
        .await
        .expect("alice sends the INVITE");
    let alice_handle =
        tokio::time::timeout(Duration::from_secs(10), alice.wait_for_answered(&call_id))
            .await
            .expect("alice timed out waiting for the answer")
            .expect("call answered");
    // Alice opens audio as soon as the call is answered.
    let alice_audio = tokio::spawn(stream_audio(
        alice_handle.clone(),
        440.0,
        300,
        Arc::clone(&failures),
    ));
    let (_bob, bob_handle, bob_audio) = bob_task.await.unwrap();

    let (alice_quality, bob_quality) = tokio::join!(
        wait_for_media(&alice_handle, Duration::from_secs(5)),
        wait_for_media(&bob_handle, Duration::from_secs(5)),
    );
    eprintln!("{keying:?} alice: {alice_quality:?}");
    eprintln!("{keying:?} bob: {bob_quality:?}");
    assert_eq!(
        failures.load(Ordering::Relaxed),
        0,
        "{keying:?}: an application send failed on a live call"
    );

    let _ = alice_handle.hangup().await;
    alice_audio.abort();
    bob_audio.abort();
}

#[cfg(feature = "dtls-srtp")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dtls_srtp_audio_opened_on_answer_flows_once_keys_are_installed() {
    audio_opened_on_answer_flows(
        Keying::Dtls,
        (37_200, 37_201),
        (37_210, 37_229),
        (37_230, 37_249),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sdes_srtp_audio_opened_on_answer_flows() {
    audio_opened_on_answer_flows(
        Keying::Sdes,
        (37_250, 37_251),
        (37_260, 37_279),
        (37_280, 37_299),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plain_rtp_audio_opened_on_answer_flows() {
    audio_opened_on_answer_flows(
        Keying::Plain,
        (37_300, 37_301),
        (37_310, 37_329),
        (37_330, 37_349),
    )
    .await;
}
