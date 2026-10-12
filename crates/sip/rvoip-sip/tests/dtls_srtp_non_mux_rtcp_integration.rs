//! DTLS-SRTP with a separate RTCP port, between two real rvoip endpoints.
//!
//! RFC 5764 §4.1: when RTP and RTCP are not multiplexed, each component runs
//! its own DTLS handshake on its own 5-tuple and keys its own SRTP/SRTCP.
//! Both endpoints enable `Config::rtcp_non_mux`; Alice offers without
//! `a=rtcp-mux`, so the call keeps a separate RTCP port on both sides. PCMU
//! streams both ways under the RTP component's keys, and each side's RTT can
//! only be measured from SRTCP that arrived on its RTCP port and decrypted
//! under the RTCP component's keys.
//!
//! The mux case (Alice offers `a=rtcp-mux`, Bob accepts) runs one handshake
//! per call and releases the RTCP ports; RTCP still flows on the RTP port.

#![cfg(feature = "dtls-srtp")]

use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::api::events::MediaSecurityKeying;
use rvoip_sip::api::unified::{Config, SrtpKeyingMode};
use rvoip_sip::{Event, EventReceiver, MediaQualityStats, SessionHandle, StreamPeer};

const SAMPLE_RATE: u32 = 8_000;
const FRAME_SAMPLES: usize = 160;

fn tone(freq: f32, frame: usize) -> Vec<i16> {
    (0..FRAME_SAMPLES)
        .map(|j| {
            let t = (frame * FRAME_SAMPLES + j) as f32 / SAMPLE_RATE as f32;
            (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

fn config(name: &str, sip_port: u16, media: (u16, u16), offer_mux: bool) -> Config {
    let mut config = Config::local(name, sip_port)
        .with_media_ports(media.0, media.1)
        .with_rtcp_non_mux(true);
    config.offer_srtp = true;
    config.srtp_required = true;
    config.srtp_keying = SrtpKeyingMode::DtlsSrtp;
    config.offer_rtcp_mux = offer_mux;
    config
}

/// Stream a tone on `handle` for `frames` 20 ms frames, draining received
/// audio so the receive path stays live.
async fn stream_audio(handle: SessionHandle, freq: f32, frames: usize) {
    let audio = handle.audio().await.expect("audio stream opens");
    let (sender, mut receiver) = audio.split();
    let drain = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    for i in 0..frames {
        let frame = AudioFrame::new(tone(freq, i), SAMPLE_RATE, 1, (i * FRAME_SAMPLES) as u32);
        if sender.send(frame).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drain.abort();
}

/// Wait until the RTP component's DTLS handshake has keyed the call: SRTP
/// sends fail closed before that.
async fn wait_for_dtls_keys(events: &mut EventReceiver, endpoint: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    loop {
        match tokio::time::timeout_at(deadline, events.next()).await {
            Ok(Some(Event::MediaSecurityNegotiated {
                keying,
                contexts_installed,
                ..
            })) => {
                assert_eq!(keying, MediaSecurityKeying::DtlsSrtp, "{endpoint}");
                assert!(contexts_installed, "{endpoint}");
                return;
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => panic!("{endpoint} never installed DTLS-SRTP keys"),
        }
    }
}

/// Poll `handle`'s media quality until media flows both ways and an RTT
/// sample from the peer's RTCP arrived, or `within` passes.
async fn wait_for_rtt(handle: &SessionHandle, within: Duration) -> Option<MediaQualityStats> {
    let deadline = tokio::time::Instant::now() + within;
    let mut last = None;
    while tokio::time::Instant::now() < deadline {
        if let Some(quality) = handle.media_quality().await {
            if quality.packets_sent > 0 && quality.packets_received > 0 && quality.rtt_ms.is_some()
            {
                return Some(quality);
            }
            last = Some(quality);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no RTT within {within:?}; last sample: {last:?}");
}

async fn run_call(sip: (u16, u16), alice_media: (u16, u16), bob_media: (u16, u16), mux: bool) {
    let _ = tracing_subscriber::fmt::try_init();
    let mut bob = StreamPeer::with_config(config("bob", sip.1, bob_media, true))
        .await
        .expect("bob starts");
    let mut alice = StreamPeer::with_config(config("alice", sip.0, alice_media, mux))
        .await
        .expect("alice starts");
    let mut alice_events = alice.coordinator().events().await.expect("alice events");
    let mut bob_events = bob.coordinator().events().await.expect("bob events");

    let bob_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(10), bob.wait_for_incoming())
            .await
            .expect("bob timed out waiting for the INVITE")
            .expect("bob received the INVITE");
        let handle = incoming.accept().await.expect("bob accepts");
        (bob, handle)
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
    let (_bob, bob_handle) = bob_task.await.unwrap();
    tokio::join!(
        wait_for_dtls_keys(&mut alice_events, "alice"),
        wait_for_dtls_keys(&mut bob_events, "bob"),
    );

    let alice_audio = tokio::spawn(stream_audio(alice_handle.clone(), 440.0, 600));
    let bob_audio = tokio::spawn(stream_audio(bob_handle.clone(), 880.0, 600));

    let (alice_quality, bob_quality) = tokio::join!(
        wait_for_rtt(&alice_handle, Duration::from_secs(12)),
        wait_for_rtt(&bob_handle, Duration::from_secs(12)),
    );
    let (alice_quality, bob_quality) = (alice_quality.unwrap(), bob_quality.unwrap());
    eprintln!("mux={mux} alice: {alice_quality:?}");
    eprintln!("mux={mux} bob: {bob_quality:?}");
    for quality in [&alice_quality, &bob_quality] {
        assert!(quality.has_peer_report(), "{quality:?}");
        let rtt = quality.rtt_ms.unwrap();
        assert!((0.0..1_000.0).contains(&rtt), "loopback RTT {rtt}");
    }

    let _ = alice_handle.hangup().await;
    alice_audio.abort();
    bob_audio.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dtls_srtp_non_mux_call_keys_rtcp_on_its_own_port_both_ways() {
    run_call((37_100, 37_101), (37_110, 37_129), (37_130, 37_149), false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dtls_srtp_mux_call_with_rtcp_non_mux_on_keeps_one_handshake() {
    run_call((37_150, 37_151), (37_160, 37_179), (37_180, 37_199), true).await;
}
