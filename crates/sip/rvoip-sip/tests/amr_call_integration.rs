//! End-to-end AMR call over loopback — two in-process endpoints, no PBX.
//!
//! Two `StreamPeer`s bind loopback SIP + RTP ports, negotiate **AMR only**
//! (no G.711 anywhere in `offered_codecs`, so a silent fallback to PCMU is
//! not available), place a real call, and stream three seconds of tone each
//! way. Each side's received PCM is then checked with a Goertzel filter for
//! the *other* side's tone, the same way `audio_roundtrip_integration.rs`
//! checks the PCMU path.
//!
//! Offering AMR alone is the point. If the negotiation quietly fell back to
//! G.711 the tone assertion would still pass, so the codec set has to make
//! that impossible rather than be asserted about afterwards — and the offer
//! Bob actually received is checked, so the claim is about the wire and not
//! about the config that produced it.
//!
//! Both variants are covered because they differ in the two places most
//! likely to be wrong: AMR-WB runs at 16 kHz with 320-sample frames, and a
//! path that assumed narrowband refuses it.

#![cfg(any(feature = "amr-nb", feature = "amr-wb"))]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::api::unified::Config;
use rvoip_sip::StreamPeer;

const FRAMES: usize = 150; // 3 s at 20 ms per frame

const ALICE_TONE_HZ: f32 = 440.0;
const BOB_TONE_HZ: f32 = 880.0;

/// Payload types this stack assigns to AMR — see `adapters/media_adapter.rs`.
/// Bandwidth-efficient framing is the RFC 4867 default and needs no fmtp.
const AMR_NB_BE_PT: u8 = 106;
const AMR_NB_OA_PT: u8 = 107;
const AMR_WB_BE_PT: u8 = 104;
const TELEPHONE_EVENT_PT: u8 = 101;

/// Allow a generous slack against the 150 frames sent, for RTP startup and
/// for the tail the receiver may not drain before teardown.
const MIN_RECEIVED_FRAMES: usize = 50;
const DOMINANCE_RATIO: f32 = 5.0;

/// One AMR variant's wire parameters, as the SIP layer negotiates them.
#[derive(Clone, Copy)]
struct AmrVariant {
    payload_type: u8,
    /// The `a=rtpmap` encoding name the offer must carry.
    rtpmap: &'static str,
    sample_rate: u32,
    /// Samples in one 20 ms frame — 160 narrowband, 320 wideband.
    frame_size: usize,
    /// The `a=fmtp` the offer must carry. `None` is a positive statement:
    /// bandwidth-efficient framing is the RFC 4867 default, so an offer
    /// that emitted one anyway would be describing a different stream.
    fmtp: Option<&'static str>,
}

const AMR_NB: AmrVariant = AmrVariant {
    payload_type: AMR_NB_BE_PT,
    rtpmap: "AMR/8000",
    sample_rate: 8_000,
    frame_size: 160,
    fmtp: None,
};

/// Octet-aligned narrowband. The framing is the one thing a peer cannot
/// recover from getting wrong, and it is the only variant here whose offer
/// carries an `a=fmtp` that has to reach the codec.
const AMR_NB_OCTET_ALIGNED: AmrVariant = AmrVariant {
    payload_type: AMR_NB_OA_PT,
    rtpmap: "AMR/8000",
    sample_rate: 8_000,
    frame_size: 160,
    fmtp: Some("octet-align=1"),
};

const AMR_WB: AmrVariant = AmrVariant {
    payload_type: AMR_WB_BE_PT,
    rtpmap: "AMR-WB/16000",
    sample_rate: 16_000,
    frame_size: 320,
    fmtp: None,
};

/// Loopback ports for one call. Each test gets a disjoint block so the two
/// can run concurrently under the default test harness.
#[derive(Clone, Copy)]
struct Ports {
    alice_sip: u16,
    bob_sip: u16,
    alice_media: (u16, u16),
    bob_media: (u16, u16),
}

fn generate_tone(variant: AmrVariant, freq: f32, frame_num: usize) -> Vec<i16> {
    (0..variant.frame_size)
        .map(|j| {
            let t = (frame_num * variant.frame_size + j) as f32 / variant.sample_rate as f32;
            (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

/// Goertzel magnitude at `target_hz`. Same filter as the PCMU roundtrip
/// test, so no new dependency is pulled in to detect a tone.
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

fn amr_only_config(
    name: &str,
    sip_port: u16,
    media: (u16, u16),
    variant: AmrVariant,
    srtp: bool,
) -> Config {
    Config {
        media_port_start: media.0,
        media_port_end: media.1,
        // AMR and DTMF only. No G.711 in the offer, so any audio that
        // arrives was carried by AMR.
        offered_codecs: vec![variant.payload_type, TELEPHONE_EVENT_PT],
        // Both sides require SRTP when it is on, so a failure to negotiate
        // fails the call rather than silently downgrading to plaintext and
        // leaving the tone assertion to pass over RTP.
        offer_srtp: srtp,
        srtp_required: srtp,
        ..Config::local(name, sip_port)
    }
}

async fn run_amr_call(variant: AmrVariant, ports: Ports) {
    run_amr_call_inner(variant, ports, false).await;
}

async fn run_amr_call_inner(variant: AmrVariant, ports: Ports, srtp: bool) {
    let _ = tracing_subscriber::fmt::try_init();

    let mut bob = StreamPeer::with_config(amr_only_config(
        "bob",
        ports.bob_sip,
        ports.bob_media,
        variant,
        srtp,
    ))
    .await
    .expect("bob peer starts with an AMR-only codec set");

    let mut alice = StreamPeer::with_config(amr_only_config(
        "alice",
        ports.alice_sip,
        ports.alice_media,
        variant,
        srtp,
    ))
    .await
    .expect("alice peer starts with an AMR-only codec set");

    let bob_received = Arc::new(Mutex::new(Vec::<i16>::new()));
    let bob_buf = bob_received.clone();
    let offer_sdp = Arc::new(Mutex::new(String::new()));
    let offer_slot = offer_sdp.clone();

    // Bob: answer, then stream 880 Hz and record whatever arrives.
    let bob_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(10), bob.wait_for_incoming())
            .await
            .expect("bob timed out waiting for the INVITE")
            .expect("bob received an incoming call");
        if let (Some(sdp), Ok(mut slot)) = (incoming.sdp.clone(), offer_slot.lock()) {
            *slot = sdp;
        }
        let handle = incoming.accept().await.expect("bob accepts the call");

        let audio = handle.audio().await.expect("bob opens the audio stream");
        let (sender, mut receiver) = audio.split();

        let recv = tokio::spawn(async move {
            while let Some(frame) = receiver.recv().await {
                if let Ok(mut buf) = bob_buf.lock() {
                    buf.extend_from_slice(&frame.samples);
                }
            }
        });

        for i in 0..FRAMES {
            let frame = AudioFrame::new(
                generate_tone(variant, BOB_TONE_HZ, i),
                variant.sample_rate,
                1,
                (i * variant.frame_size) as u32,
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

    // Alice: place the call, stream 440 Hz, record whatever arrives.
    let call_id = alice
        .invite(format!("sip:bob@127.0.0.1:{}", ports.bob_sip))
        .send()
        .await
        .expect("alice sends the INVITE");

    let handle = tokio::time::timeout(Duration::from_secs(10), alice.wait_for_answered(&call_id))
        .await
        .expect("alice timed out waiting for the answer")
        .expect("alice's AMR call was answered");

    let audio = handle.audio().await.expect("alice opens the audio stream");
    let (sender, mut receiver) = audio.split();

    let alice_received = Arc::new(Mutex::new(Vec::<i16>::new()));
    let alice_buf = alice_received.clone();
    let alice_recv = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            if let Ok(mut buf) = alice_buf.lock() {
                buf.extend_from_slice(&frame.samples);
            }
        }
    });

    for i in 0..FRAMES {
        let frame = AudioFrame::new(
            generate_tone(variant, ALICE_TONE_HZ, i),
            variant.sample_rate,
            1,
            (i * variant.frame_size) as u32,
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
    let _ = tokio::time::timeout(Duration::from_secs(10), bob_task).await;

    let alice_samples = alice_received.lock().map(|g| g.clone()).unwrap_or_default();
    let bob_samples = bob_received.lock().map(|g| g.clone()).unwrap_or_default();

    // The offer Bob actually received. This is about the wire rather than
    // the config that produced it: AMR on its dynamic payload type, and no
    // G.711 anywhere for the negotiation to quietly land on.
    let offer = offer_sdp.lock().map(|g| g.clone()).unwrap_or_default();
    eprintln!("--- {} INVITE offer as received by Bob ---\n{offer}", variant.rtpmap);
    assert!(
        offer.contains(&format!("a=rtpmap:{} {}", variant.payload_type, variant.rtpmap)),
        "the INVITE did not offer {}:\n{offer}",
        variant.rtpmap
    );
    assert!(
        !offer.contains("PCMU") && !offer.contains("PCMA"),
        "the INVITE offered G.711, so a passing audio assertion would not prove AMR:\n{offer}"
    );
    // The framing, asserted by meaning rather than by an exact string.
    // Bandwidth-efficient is the RFC 4867 default and is stated by *not*
    // saying `octet-align=1`; octet-aligned has to say so. Either way the
    // offer declines redundancy, because an absent `max-red` means no limit.
    let amr_fmtp = format!("a=fmtp:{} ", variant.payload_type);
    let fmtp_line = offer
        .lines()
        .find(|line| line.starts_with(amr_fmtp.trim_end()))
        .unwrap_or_else(|| panic!("PT {} carried no fmtp:\n{offer}", variant.payload_type));
    match variant.fmtp {
        Some(expected) => assert!(
            fmtp_line.contains(expected),
            "the INVITE did not carry `{expected}` for PT {}:\n{offer}",
            variant.payload_type
        ),
        None => assert!(
            !fmtp_line.contains("octet-align=1"),
            "PT {} is the bandwidth-efficient number and must not claim octet alignment:\n{offer}",
            variant.payload_type
        ),
    }
    assert!(
        fmtp_line.contains("max-red=0"),
        "the offer must decline redundancy unless it was asked for:\n{offer}"
    );

    // With SRTP on, the media line must be the secure profile and carry keys.
    // Without this the tone assertion would pass identically over plain RTP,
    // so the encrypted path would be untested while looking covered.
    //
    // Asserted in both positions deliberately: if the plaintext cells also
    // offered SAVP the flag would mean nothing, and every SRTP claim here
    // would be vacuous while still printing green.
    if srtp {
        assert!(
            offer.contains("RTP/SAVP"),
            "SRTP was required but the INVITE offered a plaintext profile:\n{offer}"
        );
        assert!(
            offer.contains("a=crypto:"),
            "SRTP was required but the INVITE carried no SDES key:\n{offer}"
        );
    } else {
        assert!(
            !offer.contains("RTP/SAVP") && !offer.contains("a=crypto:"),
            "the plaintext cell offered SRTP anyway, so the SRTP cells prove nothing:\n{offer}"
        );
    }

    let min_samples = MIN_RECEIVED_FRAMES * variant.frame_size;
    eprintln!(
        "{}: alice received {} samples, bob received {} samples",
        variant.rtpmap,
        alice_samples.len(),
        bob_samples.len()
    );

    assert!(
        alice_samples.len() >= min_samples,
        "alice received {} samples over {} (expected >= {})",
        alice_samples.len(),
        variant.rtpmap,
        min_samples
    );
    assert!(
        bob_samples.len() >= min_samples,
        "bob received {} samples over {} (expected >= {})",
        bob_samples.len(),
        variant.rtpmap,
        min_samples
    );

    assert!(
        alice_samples.iter().any(|&s| s != 0),
        "alice's received {} audio is pure silence",
        variant.rtpmap
    );
    assert!(
        bob_samples.iter().any(|&s| s != 0),
        "bob's received {} audio is pure silence",
        variant.rtpmap
    );

    let rate = variant.sample_rate as f32;
    let alice_peer = goertzel_magnitude(&alice_samples, rate, BOB_TONE_HZ);
    let alice_self = goertzel_magnitude(&alice_samples, rate, ALICE_TONE_HZ);
    let alice_ratio = if alice_self > 1.0 {
        alice_peer / alice_self
    } else {
        f32::INFINITY
    };
    let bob_peer = goertzel_magnitude(&bob_samples, rate, ALICE_TONE_HZ);
    let bob_self = goertzel_magnitude(&bob_samples, rate, BOB_TONE_HZ);
    let bob_ratio = if bob_self > 1.0 {
        bob_peer / bob_self
    } else {
        f32::INFINITY
    };
    eprintln!(
        "{}: alice 880Hz {alice_peer:.1} / 440Hz {alice_self:.1} = {alice_ratio:.2}; \
         bob 440Hz {bob_peer:.1} / 880Hz {bob_self:.1} = {bob_ratio:.2}",
        variant.rtpmap
    );

    assert!(
        alice_ratio >= DOMINANCE_RATIO,
        "alice's {} audio: 880Hz {:.1} vs 440Hz {:.1} (ratio {:.2}, want >= {:.2})",
        variant.rtpmap,
        alice_peer,
        alice_self,
        alice_ratio,
        DOMINANCE_RATIO
    );
    assert!(
        bob_ratio >= DOMINANCE_RATIO,
        "bob's {} audio: 440Hz {:.1} vs 880Hz {:.1} (ratio {:.2}, want >= {:.2})",
        variant.rtpmap,
        bob_peer,
        bob_self,
        bob_ratio,
        DOMINANCE_RATIO
    );
}

#[cfg(feature = "amr-nb")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn amr_narrowband_call_carries_real_audio_over_loopback() {
    run_amr_call(
        AMR_NB,
        Ports {
            alice_sip: 35_490,
            bob_sip: 35_491,
            alice_media: (35_900, 35_950),
            bob_media: (35_960, 36_010),
        },
    )
    .await;
}

/// Same call, octet-aligned. Separate because `octet-align=1` changes the
/// bit layout of every payload rather than merely its size: a peer that
/// negotiated one framing and applied the other reads garbage, so passing
/// bandwidth-efficient says nothing about this.
#[cfg(feature = "amr-nb")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn amr_octet_aligned_call_carries_real_audio_over_loopback() {
    run_amr_call(
        AMR_NB_OCTET_ALIGNED,
        Ports {
            alice_sip: 35_494,
            bob_sip: 35_495,
            alice_media: (36_140, 36_190),
            bob_media: (36_200, 36_250),
        },
    )
    .await;
}

#[cfg(feature = "amr-wb")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn amr_wideband_call_carries_real_audio_over_loopback() {
    run_amr_call(
        AMR_WB,
        Ports {
            alice_sip: 35_492,
            bob_sip: 35_493,
            alice_media: (36_020, 36_070),
            bob_media: (36_080, 36_130),
        },
    )
    .await;
}

/// The same call with SDES-SRTP required on both sides.
///
/// AMR is the codec most likely to expose an SRTP bug that G.711 cannot.
/// PCMU payloads are a fixed 160 octets every time, so a length or padding
/// mistake in the encrypt/decrypt path lands on the same size repeatedly and
/// may never be noticed. AMR payload length varies with the mode, and varies
/// again between speech, SID and NO_DATA frames once DTX is on — so the
/// authentication tag and the payload boundary move constantly.
///
/// Until now AMR-over-SRTP was evidenced only against external PBXes in the
/// interop lab, which cannot run in CI. These two run in process.
#[cfg(feature = "amr-nb")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn amr_narrowband_call_carries_real_audio_over_srtp() {
    run_amr_call_inner(
        AMR_NB,
        Ports {
            alice_sip: 35_496,
            bob_sip: 35_497,
            alice_media: (36_260, 36_310),
            bob_media: (36_320, 36_370),
        },
        true,
    )
    .await;
}

#[cfg(feature = "amr-wb")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn amr_wideband_call_carries_real_audio_over_srtp() {
    run_amr_call_inner(
        AMR_WB,
        Ports {
            alice_sip: 35_498,
            bob_sip: 35_499,
            alice_media: (36_380, 36_430),
            bob_media: (36_440, 36_490),
        },
        true,
    )
    .await;
}
