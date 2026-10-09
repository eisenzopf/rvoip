//! End-to-end per-call media quality between two real rvoip-sip endpoints.
//!
//! Two in-process `StreamPeer`s place a real call over loopback and stream
//! PCMU both ways. The application side then reads quality the two ways a
//! SIP server would: the periodic `Event::MediaQualityChanged`
//! (`Config::media_quality_interval`) and the on-demand
//! `UnifiedCoordinator::media_quality` getter.
//!
//! With RTCP multiplexing negotiated, the RTCP each side sends reaches the
//! other, so the peer-reported (`remote_*`) fields and the RTT are populated.
//! Media runs through a small UDP relay that drops every fifth RTP packet
//! Alice sends, so Bob's receiver reports must tell Alice she is losing about
//! 20 percent — the remote-reported loss is a measurement, not a constant.
//!
//! Without mux the endpoints exchange no RTCP: local statistics must still
//! arrive while every peer-reported field stays `None`.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::api::unified::Config;
use rvoip_sip::{Event, MediaQualityStats, SessionHandle, StreamPeer};
use tokio::net::UdpSocket;

const SAMPLE_RATE: u32 = 8_000;
const FRAME_SAMPLES: usize = 160;
/// One in this many RTP packets from Alice to Bob is dropped by the relay.
const DROP_EVERY: u64 = 5;

fn tone(freq: f32, frame: usize) -> Vec<i16> {
    (0..FRAME_SAMPLES)
        .map(|j| {
            let t = (frame * FRAME_SAMPLES + j) as f32 / SAMPLE_RATE as f32;
            (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

/// RFC 5761 §4: on a multiplexed port, RTCP packet types 200-204 occupy the
/// second octet that RTP uses for marker + payload type.
fn is_rtcp(datagram: &[u8]) -> bool {
    datagram.len() >= 2 && (200..=204).contains(&datagram[1])
}

/// Bidirectional media relay standing between the two endpoints.
///
/// Bob advertises `toward_bob` as his media address and Alice advertises
/// `toward_alice`, so each endpoint sends to the relay. Each side learns the
/// other's real address from the first datagram it receives, which keeps
/// the path symmetric (each endpoint sees its peer's media arrive from the
/// address it sends to). RTP from Alice to Bob is thinned by `DROP_EVERY`;
/// RTCP is never dropped.
struct LossyRelay {
    toward_bob: SocketAddr,
    toward_alice: SocketAddr,
    dropped: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl LossyRelay {
    async fn start() -> Self {
        let a_side = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let b_side = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let alice_real = Arc::new(tokio::sync::Mutex::new(None::<SocketAddr>));
        let bob_real = Arc::new(tokio::sync::Mutex::new(None::<SocketAddr>));
        let dropped = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        // Alice -> relay(a_side) -> Bob, dropping a fraction of RTP.
        {
            let (inbound, outbound) = (a_side.clone(), b_side.clone());
            let (alice_real, bob_real) = (alice_real.clone(), bob_real.clone());
            let (dropped, stop) = (dropped.clone(), stop.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 2048];
                let mut rtp_seen = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let Ok(Ok((n, from))) = tokio::time::timeout(
                        Duration::from_millis(200),
                        inbound.recv_from(&mut buf),
                    )
                    .await
                    else {
                        continue;
                    };
                    *alice_real.lock().await = Some(from);
                    let datagram = &buf[..n];
                    if !is_rtcp(datagram) {
                        rtp_seen += 1;
                        if rtp_seen % DROP_EVERY == 0 {
                            dropped.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    }
                    if let Some(bob) = *bob_real.lock().await {
                        let _ = outbound.send_to(datagram, bob).await;
                    }
                }
            });
        }
        // Bob -> relay(b_side) -> Alice, lossless.
        {
            let (inbound, outbound) = (b_side.clone(), a_side.clone());
            let (alice_real, bob_real) = (alice_real.clone(), bob_real.clone());
            let stop = stop.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 2048];
                while !stop.load(Ordering::Relaxed) {
                    let Ok(Ok((n, from))) = tokio::time::timeout(
                        Duration::from_millis(200),
                        inbound.recv_from(&mut buf),
                    )
                    .await
                    else {
                        continue;
                    };
                    *bob_real.lock().await = Some(from);
                    if let Some(alice) = *alice_real.lock().await {
                        let _ = outbound.send_to(&buf[..n], alice).await;
                    }
                }
            });
        }

        Self {
            toward_bob: a_side.local_addr().unwrap(),
            toward_alice: b_side.local_addr().unwrap(),
            dropped,
            stop,
        }
    }
}

impl Drop for LossyRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

struct Ports {
    alice_sip: u16,
    bob_sip: u16,
    alice_media: (u16, u16),
    bob_media: (u16, u16),
}

fn config(
    name: &str,
    sip_port: u16,
    media: (u16, u16),
    rtcp_mux: bool,
    advertised_media: Option<SocketAddr>,
) -> Config {
    let mut config = Config::local(name, sip_port)
        .with_media_ports(media.0, media.1)
        .with_media_quality_interval(Duration::from_secs(1));
    config.rtcp_mux_required = rtcp_mux;
    config.media_public_addr = advertised_media;
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

struct Call {
    alice: StreamPeer,
    _bob: StreamPeer,
    bob_handle: SessionHandle,
    alice_handle: SessionHandle,
    alice_events: rvoip_sip::EventReceiver,
}

async fn place_call(ports: &Ports, alice_config: Config, bob_config: Config) -> Call {
    let _ = tracing_subscriber::fmt::try_init();
    let mut bob = StreamPeer::with_config(bob_config)
        .await
        .expect("bob starts");
    let mut alice = StreamPeer::with_config(alice_config)
        .await
        .expect("alice starts");
    // Subscribe before the call so no sample can be missed.
    let alice_events = alice.coordinator().events().await.expect("alice events");

    let bob_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(10), bob.wait_for_incoming())
            .await
            .expect("bob timed out waiting for the INVITE")
            .expect("bob received the INVITE");
        let handle = incoming.accept().await.expect("bob accepts");
        (bob, handle)
    });
    let call_id = alice
        .invite(format!("sip:bob@127.0.0.1:{}", ports.bob_sip))
        .send()
        .await
        .expect("alice sends the INVITE");
    let alice_handle =
        tokio::time::timeout(Duration::from_secs(10), alice.wait_for_answered(&call_id))
            .await
            .expect("alice timed out waiting for the answer")
            .expect("call answered");
    let (bob, bob_handle) = bob_task.await.unwrap();
    Call {
        alice,
        _bob: bob,
        bob_handle,
        alice_handle,
        alice_events,
    }
}

/// Wait for a `MediaQualityChanged` on `call` that satisfies `accept`,
/// returning the last sample seen if none does before `deadline`.
async fn wait_for_quality(
    events: &mut rvoip_sip::EventReceiver,
    call: &rvoip_sip::CallId,
    within: Duration,
    mut accept: impl FnMut(&MediaQualityStats) -> bool,
) -> (bool, Option<MediaQualityStats>, usize) {
    let mut last = None;
    let mut samples = 0;
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let Ok(Some(event)) = tokio::time::timeout_at(deadline, events.next()).await else {
            return (false, last, samples);
        };
        if let Event::MediaQualityChanged {
            call_id,
            quality,
            packet_loss_percent,
            jitter_ms,
            mos,
        } = event
        {
            if &call_id != call {
                continue;
            }
            samples += 1;
            // The legacy integer fields agree with the full sample.
            assert_eq!(packet_loss_percent, quality.packet_loss_percent as u32);
            assert_eq!(jitter_ms, quality.jitter_ms as u32);
            assert_eq!(mos, quality.mos);
            let ok = accept(&quality);
            last = Some(quality);
            if ok {
                return (true, last, samples);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rtcp_mux_call_reports_local_and_peer_quality_and_measures_induced_loss() {
    let ports = Ports {
        alice_sip: 36_410,
        bob_sip: 36_411,
        alice_media: (36_420, 36_440),
        bob_media: (36_450, 36_470),
    };
    let relay = LossyRelay::start().await;
    let Call {
        alice,
        _bob,
        bob_handle,
        alice_handle,
        mut alice_events,
    } = place_call(
        &ports,
        config(
            "alice",
            ports.alice_sip,
            ports.alice_media,
            true,
            Some(relay.toward_alice),
        ),
        config(
            "bob",
            ports.bob_sip,
            ports.bob_media,
            true,
            Some(relay.toward_bob),
        ),
    )
    .await;
    let call_id = alice_handle.id().clone();

    // 12 s of media each way. RTCP reports run on a multi-second interval,
    // so this leaves room for several of them.
    let alice_audio = tokio::spawn(stream_audio(alice_handle.clone(), 440.0, 600));
    let bob_audio = tokio::spawn(stream_audio(bob_handle.clone(), 880.0, 600));

    // A sample with local counters, RTT and the full peer report.
    let (complete, sample, samples) =
        wait_for_quality(&mut alice_events, &call_id, Duration::from_secs(12), |q| {
            q.packets_sent > 0
                && q.packets_received > 0
                && q.rtt_ms.is_some()
                && q.remote_packet_loss_percent.is_some()
                && q.remote_jitter_ms.is_some()
                && q.remote_packets_lost.is_some()
        })
        .await;
    assert!(
        complete,
        "no MediaQualityChanged with RTT and peer report after {samples} samples; last: {sample:?}"
    );
    let sample = sample.unwrap();
    eprintln!("first complete sample after {samples} samples: {sample:?}");
    assert!(sample.has_peer_report());
    assert!(sample.mos.is_some(), "{sample:?}");
    let rtt = sample.rtt_ms.unwrap();
    assert!(
        (0.0..1_000.0).contains(&rtt),
        "loopback RTT out of range: {rtt}"
    );

    // Bob loses one in five of Alice's packets; his receiver reports carry
    // that back. `fraction lost` covers one RTCP interval, so accept a band
    // around 20 percent rather than an exact value.
    let (measured, loss_sample, _) =
        wait_for_quality(&mut alice_events, &call_id, Duration::from_secs(8), |q| {
            q.remote_packet_loss_percent
                .is_some_and(|loss| (12.0..=28.0).contains(&loss))
                && q.remote_packets_lost.is_some_and(|lost| lost > 0)
        })
        .await;
    assert!(
        measured,
        "remote-reported loss never reflected the relay's 20% drop: {loss_sample:?}"
    );
    eprintln!("loss sample: {loss_sample:?}");
    assert!(relay.dropped.load(Ordering::Relaxed) > 0);
    // Bob -> Alice is lossless, so Alice's own receive loss stays near zero.
    assert!(
        loss_sample.as_ref().unwrap().packet_loss_percent < 5.0,
        "{loss_sample:?}"
    );

    // The on-demand getter returns the same view, without waiting for a tick.
    let polled = alice
        .coordinator()
        .media_quality(&call_id)
        .await
        .expect("active call has media quality");
    assert!(polled.packets_sent >= sample.packets_sent);
    assert!(polled.packets_received >= sample.packets_received);
    assert!(polled.rtt_ms.is_some(), "{polled:?}");
    assert!(polled.remote_packet_loss_percent.is_some(), "{polled:?}");
    assert!(polled.remote_jitter_ms.is_some(), "{polled:?}");
    assert!(
        polled.remote_packets_lost.unwrap() >= loss_sample.unwrap().remote_packets_lost.unwrap()
    );
    // The exact-handle getter agrees with the coordinator getter.
    let via_handle = alice_handle
        .media_quality()
        .await
        .expect("handle reads the same call");
    assert!(via_handle.packets_sent >= polled.packets_sent);
    assert!(via_handle.has_peer_report());

    // Bob's own receive side observed the drop locally.
    let bob_view = bob_handle
        .media_quality()
        .await
        .expect("bob's call has media quality");
    assert!(bob_view.packets_lost > 0, "{bob_view:?}");
    assert!(
        (10.0..=30.0).contains(&bob_view.packet_loss_percent),
        "{bob_view:?}"
    );

    let _ = alice_handle.hangup().await;
    let _ = alice_audio.await;
    let _ = bob_audio.await;
    // After the call ends there is no media to report on.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(alice.coordinator().media_quality(&call_id).await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_without_rtcp_mux_reports_local_quality_and_no_peer_report() {
    let ports = Ports {
        alice_sip: 36_412,
        bob_sip: 36_413,
        alice_media: (36_480, 36_499),
        bob_media: (36_500, 36_519),
    };
    let Call {
        alice,
        _bob,
        bob_handle,
        alice_handle,
        mut alice_events,
    } = place_call(
        &ports,
        config("alice", ports.alice_sip, ports.alice_media, false, None),
        config("bob", ports.bob_sip, ports.bob_media, false, None),
    )
    .await;
    let call_id = alice_handle.id().clone();

    let alice_audio = tokio::spawn(stream_audio(alice_handle.clone(), 440.0, 400));
    let bob_audio = tokio::spawn(stream_audio(bob_handle.clone(), 880.0, 400));

    // Collect samples for long enough that RTCP, were any exchanged, would
    // have produced a peer report.
    let (_, sample, samples) =
        wait_for_quality(&mut alice_events, &call_id, Duration::from_secs(7), |_| {
            false
        })
        .await;
    assert!(samples >= 3, "expected periodic samples, got {samples}");
    let sample = sample.expect("local quality arrives without RTCP");
    eprintln!("no-mux sample after {samples} samples: {sample:?}");
    assert!(sample.packets_sent > 0, "{sample:?}");
    assert!(sample.packets_received > 0, "{sample:?}");
    assert!(sample.mos.is_some(), "{sample:?}");
    assert!(!sample.has_peer_report(), "{sample:?}");
    assert_eq!(sample.remote_packet_loss_percent, None);
    assert_eq!(sample.remote_packets_lost, None);
    assert_eq!(sample.remote_jitter_ms, None);
    assert_eq!(sample.rtt_ms, None);

    let polled = alice
        .coordinator()
        .media_quality(&call_id)
        .await
        .expect("active call has media quality");
    assert!(polled.packets_received > 0);
    assert!(!polled.has_peer_report());
    assert_eq!(polled.rtt_ms, None);

    let _ = alice_handle.hangup().await;
    let _ = alice_audio.await;
    let _ = bob_audio.await;
}

/// The same samples reach the transport-neutral layer: a SIP call placed
/// through the rvoip-core `Orchestrator` gets `MediaStream` quality and
/// `rvoip_core::Event::MediaQuality` carrying RTT and the peer's report.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn orchestrator_receives_sip_media_quality_with_peer_report() {
    use rvoip_core::adapter::OriginateRequest;
    use rvoip_core::stream::{MediaFrame, StreamKind};
    use rvoip_core::{
        CapabilityDescriptor, Config as CoreConfig, ConversationPolicy, Direction, Orchestrator,
        ParticipantId, SessionMedium, TenantId, Transport,
    };

    let _ = tracing_subscriber::fmt::try_init();
    let (alice_sip, bob_sip) = (36_414, 36_415);
    let mut bob = StreamPeer::with_config(config("bob", bob_sip, (36_530, 36_549), true, None))
        .await
        .expect("bob starts");
    let bob_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(10), bob.wait_for_incoming())
            .await
            .expect("bob timed out waiting for the INVITE")
            .expect("bob received the INVITE");
        let handle = incoming.accept().await.expect("bob accepts");
        stream_audio(handle, 880.0, 500).await;
        bob
    });

    let coordinator = rvoip_sip::UnifiedCoordinator::new(config(
        "alice",
        alice_sip,
        (36_560, 36_579),
        true,
        None,
    ))
    .await
    .expect("alice coordinator");
    let adapter = rvoip_sip::SipAdapter::new(coordinator.clone())
        .await
        .expect("sip adapter");
    let orchestrator = Orchestrator::new(CoreConfig::default());
    orchestrator.register(adapter).expect("register sip");
    let mut core_events = orchestrator.subscribe_events();
    let conversation = orchestrator
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            Default::default(),
        )
        .await
        .unwrap();
    let session = orchestrator
        .start_session(conversation, SessionMedium::Voice, vec![])
        .await
        .unwrap();
    let handle = orchestrator
        .originate_connection(OriginateRequest {
            session_id: session,
            participant_id: ParticipantId::new(),
            target: format!("sip:bob@127.0.0.1:{bob_sip}"),
            direction: Direction::Outbound,
            capabilities: CapabilityDescriptor::default(),
            transport: Some(Transport::Sip),
            context: Default::default(),
        })
        .await
        .expect("originate");
    let connection_id = handle.connection.id.clone();
    let stream = handle.connection.streams[0].clone();

    // Alice sends PCMU silence so Bob has a stream of hers to report on.
    let sender = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(sender) = stream.stream().try_frames_out() {
                return sender;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("SIP media stream never activated");
    let stream_id = stream.id();
    let alice_audio = tokio::spawn(async move {
        for i in 0..400u32 {
            let frame = MediaFrame {
                stream_id: stream_id.clone(),
                kind: StreamKind::Audio,
                payload: bytes::Bytes::from(vec![0xFFu8; FRAME_SAMPLES]),
                timestamp_rtp: i * FRAME_SAMPLES as u32,
                captured_at: chrono::Utc::now(),
                payload_type: Some(0),
            };
            if sender.send(frame).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    let snapshot = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            match core_events.recv().await {
                Ok(rvoip_core::Event::MediaQuality {
                    connection_id: id,
                    snapshot,
                    ..
                }) if id == connection_id
                    && snapshot.rtt_ms.is_some()
                    && snapshot.remote_packet_loss_pct.is_some() =>
                {
                    return snapshot;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(error) => panic!("core event stream closed: {error}"),
            }
        }
    })
    .await
    .expect("no rvoip-core MediaQuality with RTT and peer report");
    assert!(snapshot.mos.is_some(), "{snapshot:?}");
    assert!(snapshot.remote_jitter_ms.is_some(), "{snapshot:?}");
    // The stream retains the reading for pollers such as
    // `spawn_media_quality_sampler`.
    assert!(stream.stream().has_quality_measurement());
    assert!(stream.stream().quality_snapshot().rtt_ms.is_some());

    alice_audio.abort();
    for call in coordinator.list_sessions().await {
        let _ = coordinator.hangup(&call.session_id).await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(15), bob_task).await;
}
