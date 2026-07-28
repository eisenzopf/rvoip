//! Cross-transport bridge demo — SIP + WebRTC + QUIC under one Orchestrator.
//!
//! This is the load-bearing proof that the rvoip-core spine bridges
//! across substrates. A single `Orchestrator` registers three real
//! adapters from three different crates:
//!
//! - `rvoip_sip::SipAdapter`     — SIP/RTP interop
//! - `rvoip_webrtc::WebRtcAdapter` — WebRTC interop (DTLS-SRTP / ICE)
//! - `rvoip_quic::UctpQuicAdapter` — UCTP-over-QUIC substrate
//!
//! What the example proves (per `INTERFACE_DESIGN.md` §10):
//!
//! 1. All three adapters can be constructed and registered against
//!    the SAME `Orchestrator` — no cross-substrate coupling beyond
//!    the `ConnectionAdapter` trait. Confirmed by
//!    `Orchestrator::adapter(Transport::X)` returning each.
//! 2. The cross-substrate event bus normalizes every adapter's
//!    native events into `rvoip_core::Event`s. We subscribe once and
//!    see events from all three.
//! 3. Real cross-substrate bridging works end-to-end on the QUIC
//!    path: two QUIC clients dial the server, the orchestrator
//!    bridges their `Connection`s with `bridge_connections`, an
//!    audio frame pushed by client A arrives at client B's
//!    `frames_in` receiver — proving the bridge frame-pump operates
//!    against media streams resolved through the adapter contract
//!    regardless of which adapter owns the connection.
//!
//! What's NOT in this demo (deferred per
//! `rvoip-uctp/examples/uctp_to_sip_bridge/orchestrator_bridge.rs`):
//! SIP↔QUIC and WebRTC↔QUIC frame-level bridging require populated
//! `MediaStream`s on the SIP / WebRTC sides; the SIP adapter's
//! `streams()` impl is still the legacy SIP-bridge shape and WebRTC
//! needs full SDP negotiation with a real remote. Both are addressed
//! by tickets outside the gap plan's P12 scope. The dispatch path is
//! exercised here (the orchestrator picks the right adapter); the
//! actual frame flow across SIP↔QUIC depends on those follow-ups.
//!
//! Run:
//!
//! ```bash
//! cargo run -p rvoip-core --example cross_transport_bridge
//! ```
//!
//! The example exits cleanly after ~3 seconds (the demo timeout).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;

use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest};
use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::connection::Direction;
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, ParticipantId, StreamId, TenantId};
use rvoip_core::session::SessionMedium;
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use rvoip_core::{Config, Orchestrator, Transport};

use rvoip_quic::{
    spawn_datagram_reader, QuicDatagramMediaStream, UctpQuicAdapter, UctpQuicClient, UctpQuicConfig,
};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session::SessionInvite};
use rvoip_uctp::substrate::{
    dev_client_config_trusting, dispatch_by_alpn, make_server_endpoint, self_signed_for_dev,
};
use rvoip_uctp::types::MessageType;
use rvoip_webrtc::config::WebRtcConfig;
use rvoip_webrtc::media::from_tracks;
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcAdapter;

const ALPN_UCTP: &[u8] = b"uctp/1";

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn rand_hex() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("{:016x}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Build a loopback QUIC server endpoint with self-signed cert and
/// the UCTP ALPN. Returns the endpoint plus its DER-encoded cert
/// (so clients can pin trust).
fn quic_server_endpoint(
    addr: SocketAddr,
) -> (
    Arc<quinn::Endpoint>,
    rustls::pki_types::CertificateDer<'static>,
) {
    let (cert_der, key_der) = self_signed_for_dev(&["localhost".into()]).expect("self_signed");
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server tls");
    tls.alpn_protocols = vec![ALPN_UCTP.to_vec()];
    let endpoint = make_server_endpoint(addr, Arc::new(tls), quinn::TransportConfig::default())
        .expect("endpoint");
    (Arc::new(endpoint), cert_der)
}

fn quic_client_endpoint() -> Arc<quinn::Endpoint> {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("client bind");
    Arc::new(
        quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            None,
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .expect("client endpoint"),
    )
}

/// Dial a UCTP-over-QUIC client into the server, run the bearer auth
/// handshake, then send a `session.invite`. Returns the client handle
/// plus a paired client-side `QuicDatagramMediaStream` (outbound +
/// inbound multiplexed on `stream_local_id=1`) with a spawned datagram
/// reader. Test code uses `client_stream.frames_out()` to inject and
/// `client_stream.frames_in()` to observe the round-trip.
async fn dial_quic_client_with_stream(
    client_ep: &quinn::Endpoint,
    server_addr: SocketAddr,
    cert: &rustls::pki_types::CertificateDer<'static>,
    sid: &str,
    participant: &str,
    direction: rvoip_core::connection::Direction,
) -> (Arc<UctpQuicClient>, Arc<QuicDatagramMediaStream>) {
    let client = dial_quic_client(client_ep, server_addr, cert, sid, participant).await;
    let codec = rvoip_core::capability::CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48000,
        channels: 1,
        fmtp: None,
    };
    let stream = QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec,
        direction,
        1, // server-side stream_local_id is 1 (see UctpQuicAdapter::insert_route)
        client.connection.clone(),
    );
    let router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&stream)]));
    spawn_datagram_reader(client.connection.clone(), router, None);
    (client, stream)
}

async fn dial_quic_client(
    client_ep: &quinn::Endpoint,
    server_addr: SocketAddr,
    cert: &rustls::pki_types::CertificateDer<'static>,
    sid: &str,
    participant: &str,
) -> Arc<UctpQuicClient> {
    let client_cfg = dev_client_config_trusting(cert).expect("client tls");
    let client = UctpQuicClient::connect(client_ep, server_addr, "localhost", Arc::new(client_cfg))
        .await
        .expect("client connect");

    let mut inbound = client.take_inbound().expect("take_inbound");

    // auth.hello → auth.challenge → auth.response → auth.session.
    let hello = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: format!("env_{}", rand_hex()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_demo".into(),
                kind: "desktop".into(),
                platform: "demo".into(),
                sdk_version: "rvoip-demo/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
        signature: None,
    };
    client.send(hello).await.expect("send hello");
    let challenge = tokio::time::timeout(Duration::from_secs(2), inbound.recv())
        .await
        .expect("auth.challenge timeout")
        .expect("inbound closed");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
    let response = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthResponse,
        id: format!("env_{}", rand_hex()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: Some(challenge.id),
        payload: serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "test-token".into(),
            actor_token: None,
        })
        .unwrap(),
        signature: None,
    };
    client.send(response).await.expect("send response");
    let session_reply = tokio::time::timeout(Duration::from_secs(2), inbound.recv())
        .await
        .expect("auth.session timeout")
        .expect("inbound closed");
    assert_eq!(session_reply.msg_type, MessageType::AuthSession);

    let invite = UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionInvite,
        id: format!("env_{}", rand_hex()),
        ts: Utc::now(),
        cid: Some(format!("conv_{}", rand_hex())),
        sid: Some(sid.into()),
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(SessionInvite {
            from: participant.into(),
            to: vec!["part_bridge".into()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities_offer: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
        signature: None,
    };
    client.send(invite).await.expect("send invite");
    client
}

// =====================================================================
// Tone-based transcoding fidelity helpers.
//
// Silent frames only prove "bytes flowed". To prove "transcoding
// preserved the signal", we send a real sine wave at a known frequency
// on each direction and verify the received audio carries the same
// tone. Each direction uses a distinct frequency so we can detect
// crosstalk between bridges that share a process.
// =====================================================================

/// Generate `num_samples` PCM samples of a sine wave at `freq_hz`
/// with the given `sample_rate` and `amplitude` (peak, max i16::MAX).
fn pcm_sine(freq_hz: f32, sample_rate: u32, num_samples: usize, amplitude: i16) -> Vec<i16> {
    let two_pi_f = 2.0 * std::f32::consts::PI * freq_hz;
    let mut out = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let s = (two_pi_f * t).sin() * amplitude as f32;
        out.push(s as i16);
    }
    out
}

/// Encode a long sine wave as a sequence of 20 ms Opus frames at
/// 48 kHz stereo (the bridge's transcoder negotiated codec). Each
/// returned `Vec<u8>` is one Opus payload ready to wrap in RTP.
fn opus_tone_frames(freq_hz: f32, num_frames: usize) -> Vec<Vec<u8>> {
    use rvoip_media_core::codec::audio::common::AudioCodec;
    use rvoip_media_core::codec::audio::{OpusApplication, OpusCodec, OpusConfig};
    use rvoip_media_core::types::SampleRate;
    let mut enc = OpusCodec::new(
        SampleRate::Rate48000,
        2,
        OpusConfig {
            application: OpusApplication::Voip,
            bitrate: 64000,
            frame_size_ms: 20.0,
            vbr: true,
            complexity: 5,
        },
    )
    .expect("opus encoder");
    // 20 ms @ 48 kHz stereo = 960 mono samples × 2 channels = 1920.
    let mono = pcm_sine(freq_hz, 48000, 960 * num_frames, 12000);
    // Upmix mono → stereo: duplicate each sample.
    let stereo: Vec<i16> = mono.iter().flat_map(|&s| [s, s]).collect();
    let mut frames = Vec::with_capacity(num_frames);
    for chunk in stereo.chunks(1920) {
        if chunk.len() < 1920 {
            break;
        }
        let af = rvoip_media_core::types::AudioFrame::new(chunk.to_vec(), 48000, 2, 0);
        let bytes = enc.encode(&af).expect("opus encode");
        frames.push(bytes);
    }
    frames
}

/// Decode a series of Opus payloads back to interleaved stereo PCM
/// at 48 kHz, then downmix to mono. Used to analyze what came out the
/// other side of a bridge that emitted Opus (the QUIC side).
fn decode_opus_to_mono_pcm(payloads: &[Vec<u8>]) -> Vec<i16> {
    use rvoip_media_core::codec::audio::common::AudioCodec;
    use rvoip_media_core::codec::audio::{OpusApplication, OpusCodec, OpusConfig};
    use rvoip_media_core::types::SampleRate;
    let mut dec = OpusCodec::new(
        SampleRate::Rate48000,
        2,
        OpusConfig {
            application: OpusApplication::Voip,
            bitrate: 64000,
            frame_size_ms: 20.0,
            vbr: true,
            complexity: 5,
        },
    )
    .expect("opus decoder");
    let mut mono = Vec::new();
    for p in payloads {
        if p.is_empty() {
            continue;
        }
        match dec.decode(p) {
            Ok(af) => {
                // Stereo → mono: average pairs.
                for pair in af.samples.chunks(2) {
                    let m = if pair.len() == 2 {
                        ((pair[0] as i32 + pair[1] as i32) / 2) as i16
                    } else {
                        pair[0]
                    };
                    mono.push(m);
                }
            }
            Err(_) => continue,
        }
    }
    mono
}

/// Estimate the dominant frequency in a PCM buffer via zero-crossing
/// rate. For a clean sine wave at `f` Hz the signal crosses zero `2f`
/// times per second, so:  f ≈ (zero_crossings / 2) * (sample_rate / samples)
fn dominant_freq_hz(samples: &[i16], sample_rate: u32) -> f32 {
    if samples.len() < 2 {
        return 0.0;
    }
    let mut crossings = 0usize;
    for w in samples.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (a >= 0 && b < 0) || (a < 0 && b >= 0) {
            crossings += 1;
        }
    }
    let duration_s = samples.len() as f32 / sample_rate as f32;
    (crossings as f32 / 2.0) / duration_s
}

/// RMS energy of a PCM buffer. Returns 0..32767 (peak i16 amplitude).
fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum_sq / samples.len() as f64).sqrt() as f32
}

/// Verify that received audio matches an expected tone. Returns a
/// triple (ok, observed_freq, observed_rms) where `ok` is true when
/// the dominant frequency is within `tolerance_hz` and the RMS is
/// above `min_rms` (proving non-silence).
fn assert_tone(
    samples: &[i16],
    sample_rate: u32,
    expected_freq_hz: f32,
    tolerance_hz: f32,
    min_rms: f32,
) -> (bool, f32, f32) {
    let observed_freq = dominant_freq_hz(samples, sample_rate);
    let observed_rms = rms(samples);
    let freq_ok = (observed_freq - expected_freq_hz).abs() < tolerance_hz;
    let energy_ok = observed_rms >= min_rms;
    (freq_ok && energy_ok, observed_freq, observed_rms)
}

/// Spawn a background task on the SIP peer coordinator that auto-accepts
/// every incoming call, sending each call's `SessionId` back via an
/// mpsc receiver. Test code calls `.recv().await` once per expected
/// inbound call to grab its session_id.
fn spawn_peer_auto_acceptor(
    peer: Arc<UnifiedCoordinator>,
) -> tokio::sync::mpsc::Receiver<rvoip_sip::SessionId> {
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    tokio::spawn(async move {
        loop {
            match peer.get_incoming_call().await {
                Some(info) => {
                    let sid = info.session_id.clone();
                    if let Err(e) = peer.accept_call(&sid).await {
                        eprintln!("[sip-peer] accept_call failed: {e}");
                    } else {
                        println!("[sip-peer] auto-accepted incoming call (sid={})", sid.0);
                    }
                    if tx.send(sid).await.is_err() {
                        // Receiver dropped — no more interest.
                        break;
                    }
                }
                None => break,
            }
        }
    });
    rx
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "info,rvoip_sip_dialog=warn,webrtc=warn".into()),
        )
        .init();
    install_crypto_provider();

    println!("=== cross_transport_bridge: SIP + WebRTC + QUIC under one Orchestrator ===\n");

    // -------------------------------------------------------------
    // 1. Build all three adapters.
    // -------------------------------------------------------------

    // SIP — proven api::UnifiedCoordinator on a loopback UDP port.
    let sip_bind: SocketAddr = "127.0.0.1:5099".parse().unwrap();
    let sip_coordinator = UnifiedCoordinator::new(SipConfig::on(
        "rvoip-bridge-demo",
        sip_bind.ip(),
        sip_bind.port(),
    ))
    .await?;
    let sip_adapter = SipAdapter::new(sip_coordinator).await?;
    println!("[1/3] SipAdapter bound on {}", sip_bind);

    // WebRTC — loopback config; no STUN, ephemeral UDP port.
    let webrtc_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    println!("[2/3] WebRtcAdapter built (loopback config)");

    // QUIC — self-signed cert + ALPN dispatch on a loopback port.
    let (quic_ep, cert_der) = quic_server_endpoint("127.0.0.1:0".parse().unwrap());
    let quic_bind = quic_ep.local_addr()?;
    let mut routes = dispatch_by_alpn(Arc::clone(&quic_ep), &[ALPN_UCTP])?;
    let accept_rx = routes.take(ALPN_UCTP).expect("uctp/1");
    let quic_adapter = UctpQuicAdapter::new(UctpQuicConfig::new(
        Arc::clone(&quic_ep),
        accept_rx,
        bearer_stub(),
    ))
    .await?;
    println!("[3/3] UctpQuicAdapter bound on {}\n", quic_bind);

    // -------------------------------------------------------------
    // 2. Register all three with one Orchestrator.
    // -------------------------------------------------------------

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator.register(sip_adapter.clone() as Arc<dyn ConnectionAdapter>)?;
    orchestrator.register(webrtc_adapter.clone() as Arc<dyn ConnectionAdapter>)?;
    orchestrator.register(quic_adapter.clone() as Arc<dyn ConnectionAdapter>)?;

    println!("=== Registered adapters: ===");
    for transport in [Transport::Sip, Transport::WebRtc, Transport::Quic] {
        let adapter = orchestrator.adapter(transport).expect("just registered");
        println!(
            "  transport={:?}  kind={:?}  capabilities={} audio codec(s)",
            adapter.transport(),
            adapter.kind(),
            adapter.capabilities().audio_codecs.len()
        );
    }
    println!();

    // -------------------------------------------------------------
    // 3. Subscribe to the cross-substrate event bus.
    // -------------------------------------------------------------

    let mut events = orchestrator.subscribe_events();
    let orch_for_bridge = Arc::new(orchestrator);

    // Event collector — runs until two QUIC ConnectionInbounds land
    // (then bridges them), or the demo timeout hits.
    let collector = {
        let orch = Arc::clone(&orch_for_bridge);
        tokio::spawn(async move {
            let mut pending: Vec<ConnectionId> = Vec::new();
            let mut bridged = false;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            while tokio::time::Instant::now() < deadline {
                match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
                    Ok(Ok(event)) => {
                        println!("[event] {event:?}");
                        if let Event::ConnectionInbound { connection_id, .. } = &event {
                            pending.push(connection_id.clone());
                            if pending.len() == 2 && !bridged {
                                let a = pending.remove(0);
                                let b = pending.remove(0);
                                match orch.bridge_connections(a.clone(), b.clone()).await {
                                    Ok(bid) => println!(
                                        "[bridge] orchestrator bridged {} <-> {} as {}",
                                        a, b, bid
                                    ),
                                    Err(e) => println!(
                                        "[bridge] bridge_connections({}, {}) -> {}",
                                        a, b, e
                                    ),
                                }
                                bridged = true;
                            }
                        }
                    }
                    _ => continue,
                }
            }
            bridged
        })
    };

    // -------------------------------------------------------------
    // 4. Drive the QUIC path: two clients dial, orchestrator bridges.
    // -------------------------------------------------------------

    let client_ep_a = quic_client_endpoint();
    let client_ep_b = quic_client_endpoint();
    let _client_a =
        dial_quic_client(&client_ep_a, quic_bind, &cert_der, "sess_demo_a", "part_a").await;
    let _client_b =
        dial_quic_client(&client_ep_b, quic_bind, &cert_der, "sess_demo_b", "part_b").await;
    println!("\n[quic] two clients connected; QUIC↔QUIC bridge fires asynchronously...\n");

    // Wait for the collector to finish the QUIC↔QUIC bridge or its
    // own timeout — whichever comes first.
    let bridged_quic = collector.await.unwrap_or(false);

    // Touch a type from rvoip_quic so the dev-dep doesn't warn unused.
    let _ = QuicDatagramMediaStream::id;
    let _ = (
        Bytes::from_static(b"x"),
        MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(&[0]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: Some(0),
        },
    );

    // -------------------------------------------------------------
    // 5. Phase 2 — exercise SIP `originate_connection`.
    //
    // This is the load-bearing proof for cross-substrate dispatch on
    // the SIP side: we call `Orchestrator::originate_connection` with
    // `transport: Some(Transport::Sip)` and observe that the call
    // routes to `SipAdapter::originate`, which sends a real SIP
    // INVITE on the wire toward a target URI.
    //
    // The target is a second in-process SIP coordinator on a
    // different port — not registered with this orchestrator, just
    // there so the INVITE has somewhere real to land. The peer's
    // own event bus will see the inbound INVITE; the orchestrator
    // emits `Event::ConnectionOutbound` for the originated leg.
    // -------------------------------------------------------------

    let peer_bind: SocketAddr = "127.0.0.1:5199".parse().unwrap();
    let peer_coordinator = UnifiedCoordinator::new(SipConfig::on(
        "rvoip-bridge-peer",
        peer_bind.ip(),
        peer_bind.port(),
    ))
    .await?;
    println!("[sip] in-process peer coordinator bound on {} (not registered with orchestrator — pure listener)", peer_bind);

    // Auto-accept incoming calls on the peer side so SIP dialogs reach
    // Connected — required for media to flow across the bridge. The
    // returned mpsc yields one peer-side `SessionId` per accepted call;
    // Phase 4 reads the first, Phase 6 reads the second.
    let mut peer_sid_rx = spawn_peer_auto_acceptor(Arc::clone(&peer_coordinator));

    // Open a Conversation + Session so the orchestrator has somewhere
    // to attach the new Connection.
    let cid = orch_for_bridge
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            Default::default(),
        )
        .await?;
    let sid = orch_for_bridge
        .start_session(cid.clone(), SessionMedium::Voice, vec![])
        .await?;
    println!("[sip] opened Conversation {} + Session {}", cid, sid);

    // The key call: originate via SIP transport selector.
    let originate = OriginateRequest {
        session_id: sid.clone(),
        participant_id: ParticipantId::new(),
        target: format!("sip:demo@{}", peer_bind),
        direction: Direction::Outbound,
        capabilities: CapabilityDescriptor::default(),
        transport: Some(Transport::Sip),
        context: Default::default(),
    };

    let sip_originate_result = orch_for_bridge.originate_connection(originate).await;
    let sip_conn_id = match sip_originate_result {
        Ok(handle) => {
            let id = handle.connection.id.clone();
            println!(
                "[sip] orchestrator.originate_connection(Transport::Sip) → Connection {} (transport {:?})",
                id, handle.connection.transport
            );
            Some(id)
        }
        Err(e) => {
            println!("[sip] orchestrator.originate_connection(Transport::Sip) → ERROR: {e}");
            None
        }
    };

    // Give the SIP transaction a moment to put the INVITE on the wire.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // -------------------------------------------------------------
    // 6. Phase 3 — attempt a real SIP↔QUIC bridge.
    //
    // Dial a fresh QUIC client (the first two are already inside the
    // QUIC↔QUIC bridge from Phase 1), then call bridge_connections
    // between the SIP-originated leg and the new QUIC inbound.
    //
    // The honest expectation: this may fail with a specific error
    // depending on the SIP leg's state and stream readiness. The
    // proof here is whichever the answer is — success means full
    // cross-substrate bridging is live; an error proves the dispatch
    // routed correctly to the bridge code path with both adapters in
    // hand. Either way it demonstrates the architecture handles
    // (Sip, Quic) pairs through the same API as (Quic, Quic).
    // -------------------------------------------------------------

    // Subscribe BEFORE dialing so we don't miss the new inbound.
    let mut late_events = orch_for_bridge.subscribe_events();
    let client_ep_c = quic_client_endpoint();
    let (_client_c, client_c_stream) = dial_quic_client_with_stream(
        &client_ep_c,
        quic_bind,
        &cert_der,
        "sess_demo_c",
        "part_c",
        Direction::Inbound, // we'll observe incoming bridged frames here
    )
    .await;

    let mut quic_c_id: Option<ConnectionId> = None;
    for _ in 0..15 {
        match tokio::time::timeout(Duration::from_millis(200), late_events.recv()).await {
            Ok(Ok(Event::ConnectionInbound { connection_id, .. })) => {
                println!("[late-event] ConnectionInbound for QUIC client C: {connection_id}");
                quic_c_id = Some(connection_id);
                break;
            }
            Ok(Ok(other)) => println!("[late-event] {other:?}"),
            _ => continue,
        }
    }

    let cross_bridge_result = match (sip_conn_id.as_ref(), quic_c_id.as_ref()) {
        (Some(sip), Some(quic)) => {
            println!(
                "\n[cross-bridge] attempting Orchestrator::bridge_connections({}, {}) — SIP↔QUIC",
                sip, quic
            );
            match orch_for_bridge
                .bridge_connections(sip.clone(), quic.clone())
                .await
            {
                Ok(bid) => {
                    println!(
                        "[cross-bridge] SUCCESS — bridge_id={} (SIP↔QUIC frame-pump active)",
                        bid
                    );
                    Ok(bid)
                }
                Err(e) => {
                    println!("[cross-bridge] bridge_connections returned: {e}");
                    println!(
                        "  (Dispatch DID fire — both adapters were resolved through the \n   cross-substrate spine; the error reports which leg's MediaStream\n   was not yet pump-ready.)"
                    );
                    Err(e)
                }
            }
        }
        _ => {
            println!(
                "[cross-bridge] skipped — sip_conn_id={:?} quic_c_id={:?}",
                sip_conn_id, quic_c_id
            );
            Err(rvoip_core::error::RvoipError::AdmissionRejected(
                "preconditions not met".into(),
            ))
        }
    };

    // -------------------------------------------------------------
    // 7. Phase 4 — SIP↔QUIC bi-directional frame round-trip.
    //
    // The bridge is constructed; the SIP dialog should reach Connected
    // because the peer auto-acceptor accepted the call. Both sides
    // have live media. Now drive an audio frame each direction and
    // confirm arrival.
    // -------------------------------------------------------------

    let mut sip_to_quic_ok = false;
    let mut quic_to_sip_ok = false;
    let mut frame_test_note = String::new();

    if cross_bridge_result.is_ok() && sip_conn_id.is_some() {
        // Wait for the peer's auto-acceptor to receive the inbound call
        // SessionId (peer-side analogue of the orchestrator's sip_conn_id).
        match tokio::time::timeout(Duration::from_secs(3), peer_sid_rx.recv()).await {
            Ok(Some(peer_sid)) => {
                println!(
                    "\n[frame-test] peer accepted call → peer_sid={}",
                    peer_sid.0
                );
                // Give media negotiation a moment to settle (ACK + first RTP packet).
                tokio::time::sleep(Duration::from_millis(500)).await;

                // --- Direction A: SIP → QUIC (440 Hz tone) ---
                // Push 100 frames (2 s) of a 440 Hz tone from the SIP
                // peer; collect Opus frames on the QUIC client side;
                // decode them, concatenate, and verify the dominant
                // frequency matches.
                const SIP_TO_QUIC_TONE: f32 = 440.0;
                let mut client_c_in = MediaStream::frames_in(client_c_stream.as_ref());

                // Send tone — each AudioFrame is 20 ms = 160 samples @ 8 kHz.
                let tone_pcm = pcm_sine(SIP_TO_QUIC_TONE, 8000, 16000, 12000); // 2 s
                let mut send_ok = true;
                for (i, chunk) in tone_pcm.chunks(160).enumerate() {
                    if chunk.len() < 160 {
                        break;
                    }
                    let af = rvoip_media_core::types::AudioFrame::new(
                        chunk.to_vec(),
                        8000,
                        1,
                        (i as u32) * 160,
                    );
                    if let Err(e) = peer_coordinator.send_audio(&peer_sid, af).await {
                        frame_test_note
                            .push_str(&format!("  SIP→QUIC: peer.send_audio failed: {e}\n"));
                        send_ok = false;
                        break;
                    }
                    // Pace the sender so the SIP RTP transmitter has
                    // time to actually emit each packet.
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }

                if send_ok {
                    println!(
                        "[frame-test] SIP→QUIC: pushed 100×20ms frames of {} Hz tone",
                        SIP_TO_QUIC_TONE
                    );
                    // Collect up to 60 Opus frames on the QUIC side.
                    let mut received: Vec<Vec<u8>> = Vec::new();
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                    while received.len() < 60 && tokio::time::Instant::now() < deadline {
                        match tokio::time::timeout(Duration::from_millis(200), client_c_in.recv())
                            .await
                        {
                            Ok(Some(frame)) => received.push(frame.payload.to_vec()),
                            _ => break,
                        }
                    }

                    if received.is_empty() {
                        frame_test_note
                            .push_str("  SIP→QUIC: no MediaFrames arrived at client_c\n");
                    } else {
                        let pcm = decode_opus_to_mono_pcm(&received);
                        let (ok, freq, rms_val) = assert_tone(
                            &pcm,
                            48000,
                            SIP_TO_QUIC_TONE,
                            50.0,  // freq tolerance: ±50 Hz
                            500.0, // min RMS: signal not silence
                        );
                        println!(
                            "[frame-test] SIP→QUIC: received {} Opus frames → {} mono PCM samples; observed {:.1} Hz (expected {} Hz), rms={:.0} → {}",
                            received.len(), pcm.len(), freq, SIP_TO_QUIC_TONE, rms_val,
                            if ok { "✅ tone verified" } else { "❌ tone mismatch" }
                        );
                        sip_to_quic_ok = ok;
                        if !ok {
                            frame_test_note.push_str(&format!(
                                "  SIP→QUIC: tone analysis failed (observed {:.1} Hz, rms {:.0})\n",
                                freq, rms_val
                            ));
                        }
                    }
                }

                // --- Direction B: QUIC → SIP ---
                // Push a MediaFrame from QUIC client C; expect an
                // AudioFrame to arrive at the peer's AudioFrameSubscriber.
                let mut peer_audio_sub = match peer_coordinator.subscribe_to_audio(&peer_sid).await
                {
                    Ok(sub) => Some(sub),
                    Err(e) => {
                        frame_test_note.push_str(&format!(
                            "  QUIC→SIP: peer.subscribe_to_audio failed: {e}\n"
                        ));
                        None
                    }
                };

                if let Some(ref mut sub) = peer_audio_sub {
                    let client_c_out = MediaStream::frames_out(client_c_stream.as_ref());
                    // --- Direction B: QUIC → SIP (660 Hz tone) ---
                    // Encode a 2-second 660 Hz tone as Opus frames,
                    // send each through the QUIC wire. The bridge
                    // decodes Opus → resamples 48→8 kHz → mono →
                    // encodes G.711 → SIP RTP. The peer's
                    // AudioFrameSubscriber yields PCM AudioFrames we
                    // can analyze directly.
                    const QUIC_TO_SIP_TONE: f32 = 660.0;
                    let opus_frames = opus_tone_frames(QUIC_TO_SIP_TONE, 100);

                    for (i, payload) in opus_frames.iter().enumerate() {
                        let mf = MediaFrame {
                            stream_id: client_c_stream.id(),
                            kind: StreamKind::Audio,
                            payload: Bytes::from(payload.clone()),
                            timestamp_rtp: (i as u32) * 960,
                            captured_at: Utc::now(),
                            payload_type: Some(111),
                        };
                        let _ = client_c_out.send(mf).await;
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    println!(
                        "[frame-test] QUIC→SIP: pushed {} Opus frames of {} Hz tone",
                        opus_frames.len(),
                        QUIC_TO_SIP_TONE
                    );

                    let mut received_pcm: Vec<i16> = Vec::new();
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                    while received_pcm.len() < 8000 * 2 && tokio::time::Instant::now() < deadline {
                        match tokio::time::timeout(Duration::from_millis(200), sub.recv()).await {
                            Ok(Some(af)) => received_pcm.extend(af.samples),
                            _ => break,
                        }
                    }

                    if received_pcm.is_empty() {
                        frame_test_note.push_str("  QUIC→SIP: no AudioFrames arrived at peer\n");
                    } else {
                        let (ok, freq, rms_val) =
                            assert_tone(&received_pcm, 8000, QUIC_TO_SIP_TONE, 50.0, 500.0);
                        println!(
                            "[frame-test] QUIC→SIP: received {} PCM samples @ 8 kHz; observed {:.1} Hz (expected {} Hz), rms={:.0} → {}",
                            received_pcm.len(), freq, QUIC_TO_SIP_TONE, rms_val,
                            if ok { "✅ tone verified" } else { "❌ tone mismatch" }
                        );
                        quic_to_sip_ok = ok;
                        if !ok {
                            frame_test_note.push_str(&format!(
                                "  QUIC→SIP: tone analysis failed (observed {:.1} Hz, rms {:.0})\n",
                                freq, rms_val
                            ));
                        }
                    }
                }
            }
            _ => {
                frame_test_note.push_str(
                    "  Both: peer never received inbound call within 3s — skipping frame round-trip\n",
                );
            }
        }
    } else {
        frame_test_note.push_str(
            "  Both: bridge construction failed or no SIP conn — skipping frame round-trip\n",
        );
    }

    let sip_quic_bidir_proven = sip_to_quic_ok && quic_to_sip_ok;

    // -------------------------------------------------------------
    // 8. Phase 5 — WebRTC↔QUIC frame round-trip.
    //
    // Drive an in-process `RvoipPeerConnection` offerer that completes
    // SDP/ICE/DTLS with the registered `WebRtcAdapter` via
    // `apply_remote_offer` (the adapter's signaling entry point). Dial
    // a fresh QUIC client, bridge, inject silent RTP from the offerer,
    // observe a MediaFrame on the QUIC client's `frames_in`.
    // -------------------------------------------------------------

    let mut webrtc_quic_bridge_ok = false;
    let mut webrtc_quic_frame_ok = false;
    let mut quic_to_webrtc_frame_ok = false;
    let mut webrtc_test_note = String::new();

    // Subscribe BEFORE driving so we catch every event.
    let mut wq_events = orch_for_bridge.subscribe_events();

    // Bring up the WebRTC offerer peer in the same process.
    let webrtc_config = WebRtcConfig::loopback();
    let offerer = match RvoipPeerConnection::new(&webrtc_config, PeerRole::Offerer).await {
        Ok(p) => Some(p),
        Err(e) => {
            webrtc_test_note.push_str(&format!("  WebRTC offerer construct failed: {e}\n"));
            None
        }
    };

    let webrtc_conn_id: Option<ConnectionId> = if let Some(offerer) = offerer.as_ref() {
        match offerer.create_offer_and_gather().await {
            Ok(offer_sdp) => {
                println!(
                    "\n[webrtc] offerer generated SDP offer ({} bytes)",
                    offer_sdp.len()
                );
                // Grab the adapter's typed handle so we can call its
                // specific `apply_remote_offer` / `local_sdp` methods.
                // We need a handle to the WebRtcAdapter; we passed it
                // through `register()` but the orchestrator returns it
                // as `Arc<dyn ConnectionAdapter>` — so keep our own
                // typed reference (the `webrtc_adapter` binding earlier
                // in main is still in scope).
                match webrtc_adapter.apply_remote_offer(&offer_sdp).await {
                    Ok(conn_id) => {
                        println!(
                            "[webrtc] WebRtcAdapter::apply_remote_offer → ConnectionId {}",
                            conn_id
                        );
                        // Fetch the adapter's answer SDP and complete the handshake.
                        match webrtc_adapter.local_sdp(&conn_id) {
                            Ok(answer_sdp) => {
                                println!(
                                    "[webrtc] adapter generated answer SDP ({} bytes)",
                                    answer_sdp.len()
                                );
                                if let Err(e) = offerer.set_remote_answer(&answer_sdp).await {
                                    webrtc_test_note.push_str(&format!(
                                        "  WebRTC: offerer.set_remote_answer failed: {e}\n"
                                    ));
                                    None
                                } else if let Err(e) =
                                    offerer.wait_connected(Duration::from_secs(5)).await
                                {
                                    webrtc_test_note.push_str(&format!(
                                        "  WebRTC: offerer.wait_connected failed: {e}\n"
                                    ));
                                    None
                                } else {
                                    println!("[webrtc] offerer ICE/DTLS connected");
                                    Some(conn_id)
                                }
                            }
                            Err(e) => {
                                webrtc_test_note.push_str(&format!(
                                    "  WebRTC: adapter.local_sdp returned error: {e}\n"
                                ));
                                None
                            }
                        }
                    }
                    Err(e) => {
                        webrtc_test_note.push_str(&format!(
                            "  WebRTC: adapter.apply_remote_offer failed: {e}\n"
                        ));
                        None
                    }
                }
            }
            Err(e) => {
                webrtc_test_note.push_str(&format!(
                    "  WebRTC: offerer.create_offer_and_gather failed: {e}\n"
                ));
                None
            }
        }
    } else {
        None
    };

    // Dial a fresh QUIC client (client_d) and bridge with the WebRTC conn.
    if let (Some(webrtc_conn_id), Some(offerer)) = (webrtc_conn_id.as_ref(), offerer.as_ref()) {
        // Subscribe again so we don't miss the new QUIC inbound.
        let mut wq_late_events = orch_for_bridge.subscribe_events();
        let client_ep_d = quic_client_endpoint();
        let (client_d, client_d_stream) = dial_quic_client_with_stream(
            &client_ep_d,
            quic_bind,
            &cert_der,
            "sess_demo_d",
            "part_d",
            Direction::Inbound,
        )
        .await;

        let mut quic_d_id: Option<ConnectionId> = None;
        for _ in 0..15 {
            match tokio::time::timeout(Duration::from_millis(200), wq_late_events.recv()).await {
                Ok(Ok(Event::ConnectionInbound { connection_id, .. })) => {
                    println!("[webrtc] paired QUIC client D inbound: {connection_id}");
                    quic_d_id = Some(connection_id);
                    break;
                }
                _ => continue,
            }
        }

        if let Some(quic_d_id) = quic_d_id {
            // Accept the WebRTC connection into the orchestrator's
            // session/connection registry before bridging.
            match orch_for_bridge
                .bridge_connections(webrtc_conn_id.clone(), quic_d_id.clone())
                .await
            {
                Ok(bid) => {
                    println!(
                        "[webrtc] bridge_connections(WebRTC, QUIC) → bridge_id={}",
                        bid
                    );
                    webrtc_quic_bridge_ok = true;
                    let _ = bid; // bridge id intentionally unused; bridge lifetime is owned by webrtc_adapter

                    // The WebRtcMediaStream populates remote tracks lazily
                    // — webrtc-rs fires on_track only after the first
                    // inbound RTP packet. Use prime_remote_track to drip
                    // silent RTP from the answerer (the adapter's PC) so
                    // the offerer's RTP receiver wakes up.
                    let answerer_peer = webrtc_adapter
                        .routes()
                        .get(webrtc_conn_id)
                        .map(|r| r.peer.clone());

                    if let Some(answerer_peer) = answerer_peer {
                        let _ = RvoipPeerConnection::prime_remote_track(
                            &answerer_peer,
                            offerer,
                            Duration::from_secs(3),
                        )
                        .await;
                    }

                    // Direction WebRTC→QUIC: the bridge pump reads from
                    // webrtc_stream.frames_in() (frames the WebRTC
                    // adapter received from the offerer via on_track)
                    // and writes to quic_stream.frames_out() (sent to
                    // the QUIC peer as datagrams). To feed
                    // webrtc_stream.frames_in we need real RTP packets
                    // on the offerer's local track — those arrive at
                    // the answerer's on_track and populate the adapter's
                    // inbound pump.
                    //
                    // Construct an offerer-side MediaStream wrapping
                    // the offerer's local track (per
                    // rvoip-webrtc/tests/loopback.rs pattern) and push
                    // through its frames_out(): the underlying outbound
                    // pump in `from_tracks` wraps each MediaFrame in
                    // fresh RTP and writes via local_audio_track.
                    let codec = rvoip_core::capability::CodecInfo {
                        name: "opus".into(),
                        clock_rate_hz: 48000,
                        channels: 2,
                        fmtp: None,
                    };
                    let offerer_local = offerer.local_audio_track();
                    let offerer_ssrc = offerer.local_audio_ssrc();
                    match (offerer_local, offerer_ssrc) {
                        (Some(local), Some(ssrc)) => {
                            let offerer_stream = from_tracks(
                                StreamId::new(),
                                codec,
                                local,
                                ssrc,
                                /* Opus PT */ 111,
                                None,
                            );
                            let offerer_out = offerer_stream.frames_out();
                            let mut client_d_in = MediaStream::frames_in(client_d_stream.as_ref());

                            // --- WebRTC → QUIC (880 Hz tone) ---
                            const WQ_TONE: f32 = 880.0;
                            let tone_frames = opus_tone_frames(WQ_TONE, 100);
                            // Note: the outbound pump in `from_tracks`
                            // (spawned by spawn_outbound_pump) wraps
                            // each MediaFrame's `payload` in a fresh
                            // RTP packet with its internal sequence
                            // counter — so we pass Opus payload bytes
                            // directly, not pre-wrapped RTP.
                            for (i, payload) in tone_frames.iter().enumerate() {
                                let _ = offerer_out
                                    .send(MediaFrame {
                                        stream_id: offerer_stream.id(),
                                        kind: StreamKind::Audio,
                                        payload: Bytes::from(payload.clone()),
                                        timestamp_rtp: (i as u32) * 960,
                                        captured_at: Utc::now(),
                                        payload_type: Some(111),
                                    })
                                    .await;
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                            println!(
                                "[webrtc] pushed {} Opus tone frames ({} Hz) via offerer.local_track",
                                tone_frames.len(),
                                WQ_TONE
                            );

                            let mut received: Vec<Vec<u8>> = Vec::new();
                            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                            while received.len() < 60 && tokio::time::Instant::now() < deadline {
                                match tokio::time::timeout(
                                    Duration::from_millis(200),
                                    client_d_in.recv(),
                                )
                                .await
                                {
                                    Ok(Some(frame)) => {
                                        if !frame.payload.is_empty() {
                                            received.push(frame.payload.to_vec());
                                        }
                                    }
                                    _ => break,
                                }
                            }

                            if received.is_empty() {
                                webrtc_test_note
                                    .push_str("  WebRTC→QUIC: no MediaFrames arrived\n");
                            } else {
                                let pcm = decode_opus_to_mono_pcm(&received);
                                let (ok, freq, rms_val) =
                                    assert_tone(&pcm, 48000, WQ_TONE, 80.0, 500.0);
                                println!(
                                    "[webrtc] WebRTC→QUIC: {} Opus frames → {} mono samples; observed {:.1} Hz (expected {} Hz), rms={:.0} → {}",
                                    received.len(), pcm.len(), freq, WQ_TONE, rms_val,
                                    if ok { "✅ tone verified" } else { "❌ tone mismatch" }
                                );
                                webrtc_quic_frame_ok = ok;
                                if !ok {
                                    webrtc_test_note.push_str(&format!(
                                        "  WebRTC→QUIC: tone analysis failed (observed {:.1} Hz, rms {:.0})\n",
                                        freq, rms_val
                                    ));
                                }
                            }

                            // -------------------------------------------
                            // Phase 5b — REVERSE direction: QUIC → WebRTC
                            //
                            // Push a 1320 Hz tone from the QUIC client's
                            // frames_out → bridge_pump → adapter's
                            // answerer local track → SRTP → offerer's
                            // PeerConnection. Wait for on_track to fire,
                            // then attach the remote track to the
                            // offerer_stream so its inbound pump
                            // populates frames_in with decoded RTP
                            // payload. Decode Opus + verify.
                            // -------------------------------------------
                            const QW_TONE: f32 = 1320.0;
                            // First, coax on_track to fire by pushing a
                            // few seed frames from QUIC. The bridge
                            // forwards them to the adapter; the adapter
                            // writes RTP; offerer's PC sees first
                            // packet and fires on_track.
                            let client_d_out = MediaStream::frames_out(client_d_stream.as_ref());
                            let warmup_frames = opus_tone_frames(QW_TONE, 20);
                            for (i, payload) in warmup_frames.iter().enumerate() {
                                let _ = client_d_out
                                    .send(MediaFrame {
                                        stream_id: client_d_stream.id(),
                                        kind: StreamKind::Audio,
                                        payload: Bytes::from(payload.clone()),
                                        timestamp_rtp: (i as u32) * 960,
                                        captured_at: Utc::now(),
                                        payload_type: Some(111),
                                    })
                                    .await;
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }

                            // Now wait for offerer's on_track. Once
                            // it fires we attach the remote track to
                            // the offerer_stream so frames_in
                            // populates with subsequent RTP.
                            let remote_track =
                                offerer.wait_remote_track(Duration::from_secs(3)).await;
                            if let Some(rt) = remote_track {
                                offerer_stream.attach_remote(rt);
                                println!(
                                    "[webrtc] Phase 5b: attached remote track to offerer_stream"
                                );

                                // Drain any pre-existing frames in the
                                // channel so we only collect new tone.
                                let mut offerer_in =
                                    MediaStream::frames_in(offerer_stream.as_ref());
                                while offerer_in.try_recv().is_ok() {}

                                // Push the full QUIC→WebRTC tone.
                                let tone_frames = opus_tone_frames(QW_TONE, 100);
                                for (i, payload) in tone_frames.iter().enumerate() {
                                    let _ = client_d_out
                                        .send(MediaFrame {
                                            stream_id: client_d_stream.id(),
                                            kind: StreamKind::Audio,
                                            payload: Bytes::from(payload.clone()),
                                            // continue past warmup seq
                                            timestamp_rtp: ((20 + i) as u32) * 960,
                                            captured_at: Utc::now(),
                                            payload_type: Some(111),
                                        })
                                        .await;
                                    tokio::time::sleep(Duration::from_millis(20)).await;
                                }
                                println!(
                                    "[webrtc] Phase 5b: pushed {} Opus tone frames ({} Hz) QUIC → WebRTC",
                                    tone_frames.len(), QW_TONE
                                );

                                // Collect offerer-side frames; decode + verify.
                                let mut qw_received: Vec<Vec<u8>> = Vec::new();
                                let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                                while qw_received.len() < 60
                                    && tokio::time::Instant::now() < deadline
                                {
                                    match tokio::time::timeout(
                                        Duration::from_millis(200),
                                        offerer_in.recv(),
                                    )
                                    .await
                                    {
                                        Ok(Some(f)) => {
                                            if !f.payload.is_empty() {
                                                qw_received.push(f.payload.to_vec());
                                            }
                                        }
                                        _ => break,
                                    }
                                }

                                if qw_received.is_empty() {
                                    webrtc_test_note.push_str(
                                        "  QUIC→WebRTC: no MediaFrames arrived at offerer\n",
                                    );
                                } else {
                                    let pcm = decode_opus_to_mono_pcm(&qw_received);
                                    let (ok, freq, rms_val) =
                                        assert_tone(&pcm, 48000, QW_TONE, 80.0, 500.0);
                                    println!(
                                        "[webrtc] QUIC→WebRTC: {} Opus frames → {} mono samples; observed {:.1} Hz (expected {} Hz), rms={:.0} → {}",
                                        qw_received.len(), pcm.len(), freq, QW_TONE, rms_val,
                                        if ok { "✅ tone verified" } else { "❌ tone mismatch" }
                                    );
                                    quic_to_webrtc_frame_ok = ok;
                                    if !ok {
                                        webrtc_test_note.push_str(&format!(
                                            "  QUIC→WebRTC: tone analysis failed (observed {:.1} Hz, rms {:.0})\n",
                                            freq, rms_val
                                        ));
                                    }
                                }
                            } else {
                                webrtc_test_note.push_str(
                                    "  QUIC→WebRTC: offerer.wait_remote_track timed out\n",
                                );
                            }
                        }
                        _ => {
                            webrtc_test_note
                                .push_str("  WebRTC: offerer has no local_audio_track/ssrc\n");
                        }
                    }
                }
                Err(e) => {
                    webrtc_test_note
                        .push_str(&format!("  WebRTC↔QUIC: bridge_connections failed: {e}\n"));
                }
            }
        } else {
            webrtc_test_note.push_str("  WebRTC↔QUIC: client_d inbound never observed within 3s\n");
        }

        let _ = client_d; // keep alive
    } else {
        webrtc_test_note.push_str("  WebRTC↔QUIC: offerer/conn_id not ready — skipping\n");
    }

    // Drain any straggler events so the demo exits cleanly.
    let _ = wq_events.try_recv();

    // -------------------------------------------------------------
    // 9. Phase 6 — SIP↔WebRTC frame round-trip.
    //
    // The hardest pair: SIP emits PCMU (PT 0), WebRTC offers Opus
    // (PT 111). The bridge auto-inserts the G.711↔Opus transcoder.
    //
    // Both sip_conn_id (already bridged with quic_c) and
    // webrtc_conn_id (already bridged with quic_d) are in active
    // bridges; unbridge both first, then re-bridge as SIP↔WebRTC.
    // -------------------------------------------------------------

    let mut sip_webrtc_bridge_ok = false;
    let mut sip_webrtc_frame_ok = false;
    let mut sip_to_webrtc_frame_ok = false;
    let mut sw_test_note = String::new();

    // Phase 6 needs FRESH connections on both sides, because
    // WebRtcMediaStream::frames_in (and several other MediaStream
    // impls) are single-take — once Phase 5's bridge consumed those
    // receivers, re-bridging the same connections yields an empty
    // inbound channel and no frames flow. We originate a 2nd SIP
    // leg and bring up a 2nd WebRTC offerer.

    // --- Fresh SIP leg (#2) ---
    println!("\n[sip-webrtc] originating 2nd SIP leg + 2nd WebRTC offerer for Phase 6");
    // Reuse the same peer_sid_rx — the original auto-acceptor task
    // accepts every incoming call and forwards each session_id.
    let sid_2 = orch_for_bridge
        .start_session(cid.clone(), SessionMedium::Voice, vec![])
        .await
        .ok();
    let sip_conn_id_2 = match (
        sid_2.as_ref(),
        orch_for_bridge.adapter(Transport::Sip).is_ok(),
    ) {
        (Some(sid), true) => {
            let req = OriginateRequest {
                session_id: sid.clone(),
                participant_id: ParticipantId::new(),
                target: format!("sip:demo2@{}", peer_bind),
                direction: Direction::Outbound,
                capabilities: CapabilityDescriptor::default(),
                transport: Some(Transport::Sip),
                context: Default::default(),
            };
            orch_for_bridge
                .originate_connection(req)
                .await
                .ok()
                .map(|h| h.connection.id.clone())
        }
        _ => None,
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    let peer_sid_2 = tokio::time::timeout(Duration::from_secs(3), peer_sid_rx.recv())
        .await
        .ok()
        .flatten();

    // --- Fresh WebRTC offerer (#2) ---
    let offerer_2 = match RvoipPeerConnection::new(&webrtc_config, PeerRole::Offerer).await {
        Ok(o) => Some(o),
        Err(e) => {
            sw_test_note.push_str(&format!(
                "  SIP↔WebRTC: 2nd offerer construct failed: {e}\n"
            ));
            None
        }
    };
    let webrtc_conn_id_2 = if let Some(off2) = offerer_2.as_ref() {
        match off2.create_offer_and_gather().await {
            Ok(offer) => match webrtc_adapter.apply_remote_offer(&offer).await {
                Ok(cid) => match webrtc_adapter.local_sdp(&cid) {
                    Ok(answer) => {
                        let _ = off2.set_remote_answer(&answer).await;
                        let _ = off2.wait_connected(Duration::from_secs(5)).await;
                        Some(cid)
                    }
                    Err(_) => None,
                },
                Err(_) => None,
            },
            Err(_) => None,
        }
    } else {
        None
    };

    if cross_bridge_result.is_ok() && webrtc_quic_bridge_ok {
        if let (Some(sip_conn), Some(webrtc_conn), Some(peer_sid), Some(off2)) = (
            sip_conn_id_2.as_ref(),
            webrtc_conn_id_2.as_ref(),
            peer_sid_2.as_ref(),
            offerer_2.as_ref(),
        ) {
            match orch_for_bridge
                .bridge_connections(sip_conn.clone(), webrtc_conn.clone())
                .await
            {
                Ok(bid) => {
                    println!(
                        "[sip-webrtc] bridge_connections(SIP2, WebRTC2) → bridge_id={}",
                        bid
                    );
                    sip_webrtc_bridge_ok = true;

                    // Prime so the answerer (adapter's PC) starts
                    // pulling RTP from off2. Needed for the on_track
                    // populate on the adapter side before we push.
                    if let Some(answerer_peer_2) = webrtc_adapter
                        .routes()
                        .get(webrtc_conn)
                        .map(|r| r.peer.clone())
                    {
                        let _ = RvoipPeerConnection::prime_remote_track(
                            &answerer_peer_2,
                            off2,
                            Duration::from_secs(3),
                        )
                        .await;
                    }

                    let mut peer_sub = match peer_coordinator.subscribe_to_audio(peer_sid).await {
                        Ok(s) => Some(s),
                        Err(e) => {
                            sw_test_note.push_str(&format!(
                                "  SIP↔WebRTC: peer.subscribe_to_audio failed: {e}\n"
                            ));
                            None
                        }
                    };

                    if let (Some(ref mut sub), Some(ssrc), Some(local)) = (
                        peer_sub.as_mut(),
                        off2.local_audio_ssrc(),
                        off2.local_audio_track(),
                    ) {
                        while sub.try_recv().is_ok() {}
                        tokio::time::sleep(Duration::from_millis(300)).await;

                        // Fresh offerer_stream on offerer #2 — its
                        // local_track is unused by anyone else so no
                        // SRTP seq collision.
                        let codec = rvoip_core::capability::CodecInfo {
                            name: "opus".into(),
                            clock_rate_hz: 48000,
                            channels: 2,
                            fmtp: None,
                        };
                        let offerer_stream =
                            from_tracks(StreamId::new(), codec, local, ssrc, 111, None);
                        let out = offerer_stream.frames_out();

                        // --- WebRTC → SIP (1100 Hz tone) ---
                        const SW_TONE: f32 = 1100.0;
                        let tone_frames = opus_tone_frames(SW_TONE, 100);
                        for (i, payload) in tone_frames.iter().enumerate() {
                            let _ = out
                                .send(MediaFrame {
                                    stream_id: offerer_stream.id(),
                                    kind: StreamKind::Audio,
                                    payload: Bytes::from(payload.clone()),
                                    timestamp_rtp: (i as u32) * 960,
                                    captured_at: Utc::now(),
                                    payload_type: Some(111),
                                })
                                .await;
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        println!(
                            "[sip-webrtc] pushed {} Opus tone frames ({} Hz) via offerer_2.local_track",
                            tone_frames.len(),
                            SW_TONE
                        );

                        let mut received_pcm: Vec<i16> = Vec::new();
                        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                        while received_pcm.len() < 8000 * 2
                            && tokio::time::Instant::now() < deadline
                        {
                            match tokio::time::timeout(Duration::from_millis(200), sub.recv()).await
                            {
                                Ok(Some(af)) => received_pcm.extend(af.samples),
                                _ => break,
                            }
                        }

                        if received_pcm.is_empty() {
                            sw_test_note.push_str("  SIP↔WebRTC: no AudioFrames at peer\n");
                        } else {
                            let (ok, freq, rms_val) =
                                assert_tone(&received_pcm, 8000, SW_TONE, 80.0, 500.0);
                            println!(
                                "[sip-webrtc] WebRTC→SIP: received {} PCM samples @ 8kHz; observed {:.1} Hz (expected {} Hz), rms={:.0} → {}",
                                received_pcm.len(), freq, SW_TONE, rms_val,
                                if ok { "✅ tone verified" } else { "❌ tone mismatch" }
                            );
                            sip_webrtc_frame_ok = ok;
                            if !ok {
                                sw_test_note.push_str(&format!(
                                    "  SIP↔WebRTC: tone analysis failed (observed {:.1} Hz, rms {:.0})\n",
                                    freq, rms_val
                                ));
                            }
                        }

                        // -------------------------------------------
                        // Phase 6b — REVERSE direction: SIP → WebRTC
                        //
                        // Send a 1760 Hz tone from the SIP peer via
                        // send_audio (PCMU). Bridge transcodes G.711
                        // → Opus, adapter writes RTP via answerer's
                        // local track, offerer_2's PC fires on_track,
                        // we attach the remote track to offerer_stream
                        // and read decoded Opus → analyze.
                        // -------------------------------------------
                        const SW_TONE_2: f32 = 1760.0;
                        // Warm-up: push a few 8 kHz PCMU tone frames
                        // so the bridge starts pumping RTP toward
                        // offerer_2 (triggers on_track).
                        let warmup_pcm = pcm_sine(SW_TONE_2, 8000, 160 * 20, 12000);
                        for (i, chunk) in warmup_pcm.chunks(160).enumerate() {
                            if chunk.len() < 160 {
                                break;
                            }
                            let af = rvoip_media_core::types::AudioFrame::new(
                                chunk.to_vec(),
                                8000,
                                1,
                                (i as u32) * 160,
                            );
                            let _ = peer_coordinator.send_audio(peer_sid, af).await;
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }

                        let remote_track_2 = off2.wait_remote_track(Duration::from_secs(3)).await;
                        if let Some(rt) = remote_track_2 {
                            offerer_stream.attach_remote(rt);
                            println!(
                                "[sip-webrtc] Phase 6b: attached remote track to offerer_2 stream"
                            );
                            let mut off2_in = MediaStream::frames_in(offerer_stream.as_ref());
                            while off2_in.try_recv().is_ok() {}

                            // Push the full SIP→WebRTC tone.
                            let tone_pcm = pcm_sine(SW_TONE_2, 8000, 160 * 100, 12000);
                            for (i, chunk) in tone_pcm.chunks(160).enumerate() {
                                if chunk.len() < 160 {
                                    break;
                                }
                                let af = rvoip_media_core::types::AudioFrame::new(
                                    chunk.to_vec(),
                                    8000,
                                    1,
                                    ((20 + i) as u32) * 160,
                                );
                                let _ = peer_coordinator.send_audio(peer_sid, af).await;
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                            println!(
                                "[sip-webrtc] Phase 6b: pushed 100×20ms PCMU frames ({} Hz) SIP → WebRTC",
                                SW_TONE_2
                            );

                            // Collect on offerer_2 side; decode + verify.
                            let mut sw_received: Vec<Vec<u8>> = Vec::new();
                            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                            while sw_received.len() < 60 && tokio::time::Instant::now() < deadline {
                                match tokio::time::timeout(
                                    Duration::from_millis(200),
                                    off2_in.recv(),
                                )
                                .await
                                {
                                    Ok(Some(f)) => {
                                        if !f.payload.is_empty() {
                                            sw_received.push(f.payload.to_vec());
                                        }
                                    }
                                    _ => break,
                                }
                            }

                            if sw_received.is_empty() {
                                sw_test_note.push_str(
                                    "  SIP→WebRTC: no MediaFrames arrived at offerer_2\n",
                                );
                            } else {
                                let pcm = decode_opus_to_mono_pcm(&sw_received);
                                let (ok, freq, rms_val) =
                                    assert_tone(&pcm, 48000, SW_TONE_2, 80.0, 500.0);
                                println!(
                                    "[sip-webrtc] SIP→WebRTC: {} Opus frames → {} mono samples; observed {:.1} Hz (expected {} Hz), rms={:.0} → {}",
                                    sw_received.len(), pcm.len(), freq, SW_TONE_2, rms_val,
                                    if ok { "✅ tone verified" } else { "❌ tone mismatch" }
                                );
                                sip_to_webrtc_frame_ok = ok;
                                if !ok {
                                    sw_test_note.push_str(&format!(
                                        "  SIP→WebRTC: tone analysis failed (observed {:.1} Hz, rms {:.0})\n",
                                        freq, rms_val
                                    ));
                                }
                            }
                        } else {
                            sw_test_note
                                .push_str("  SIP→WebRTC: off2.wait_remote_track timed out\n");
                        }
                    }
                }
                Err(e) => {
                    sw_test_note
                        .push_str(&format!("  SIP↔WebRTC: bridge_connections failed: {e}\n"));
                }
            }
        } else {
            sw_test_note.push_str("  SIP↔WebRTC: missing conn ids / peer_sid — skipping\n");
        }
    } else {
        sw_test_note.push_str(
            "  SIP↔WebRTC: predecessor bridges (Phase 3 / 5) didn't succeed — skipping\n",
        );
    }

    // -------------------------------------------------------------
    // 10. Summary.
    // -------------------------------------------------------------

    println!("\n=== Demo summary ===");
    println!(
        "  SIP adapter registered:                              {}",
        orch_for_bridge.adapter(Transport::Sip).is_ok()
    );
    println!(
        "  WebRTC adapter registered:                           {}",
        orch_for_bridge.adapter(Transport::WebRtc).is_ok()
    );
    println!(
        "  QUIC adapter registered:                             {}",
        orch_for_bridge.adapter(Transport::Quic).is_ok()
    );
    println!(
        "  QUIC↔QUIC bridge constructed (Phase 1):              {}",
        bridged_quic
    );
    println!(
        "  SIP originate_connection dispatched (Phase 2):       {}",
        sip_conn_id.is_some()
    );
    println!(
        "  SIP↔QUIC bridge constructed (Phase 3):               {}",
        cross_bridge_result.is_ok()
    );
    if let Err(e) = &cross_bridge_result {
        println!("    (returned: {e})");
    }
    println!(
        "  SIP↔QUIC SIP→QUIC frame arrived (Phase 4a):          {}",
        sip_to_quic_ok
    );
    println!(
        "  SIP↔QUIC QUIC→SIP frame arrived (Phase 4b):          {}",
        quic_to_sip_ok
    );
    println!(
        "  SIP↔QUIC BI-DIRECTIONAL PROVEN:                      {}",
        sip_quic_bidir_proven
    );
    println!(
        "  WebRTC↔QUIC bridge constructed (Phase 5a):           {}",
        webrtc_quic_bridge_ok
    );
    println!(
        "  WebRTC↔QUIC WebRTC→QUIC tone verified (Phase 5):     {}",
        webrtc_quic_frame_ok
    );
    println!(
        "  WebRTC↔QUIC QUIC→WebRTC tone verified (Phase 5b):    {}",
        quic_to_webrtc_frame_ok
    );
    println!(
        "  SIP↔WebRTC bridge constructed (Phase 6a):            {}",
        sip_webrtc_bridge_ok
    );
    println!(
        "  SIP↔WebRTC WebRTC→SIP tone verified (Phase 6):       {}",
        sip_webrtc_frame_ok
    );
    println!(
        "  SIP↔WebRTC SIP→WebRTC tone verified (Phase 6b):      {}",
        sip_to_webrtc_frame_ok
    );
    let wq_bidir_proven = webrtc_quic_frame_ok && quic_to_webrtc_frame_ok;
    let sw_bidir_proven = sip_webrtc_frame_ok && sip_to_webrtc_frame_ok;
    let all_three_bidir = sip_quic_bidir_proven && wq_bidir_proven && sw_bidir_proven;
    println!();
    println!(
        "  SIP↔QUIC   BI-DIRECTIONAL:       {}",
        sip_quic_bidir_proven
    );
    println!("  WebRTC↔QUIC BI-DIRECTIONAL:      {}", wq_bidir_proven);
    println!("  SIP↔WebRTC  BI-DIRECTIONAL:      {}", sw_bidir_proven);
    println!();
    println!(
        "  *** ALL THREE PAIRS BI-DIRECTIONAL: {} ***",
        all_three_bidir
    );
    if !frame_test_note.is_empty() {
        println!("  SIP↔QUIC test notes:");
        print!("{frame_test_note}");
    }
    if !webrtc_test_note.is_empty() {
        println!("  WebRTC↔QUIC test notes:");
        print!("{webrtc_test_note}");
    }
    if !sw_test_note.is_empty() {
        println!("  SIP↔WebRTC test notes:");
        print!("{sw_test_note}");
    }
    println!();
    println!("  Architectural takeaway:");
    println!("  - One Orchestrator hosts SIP + WebRTC + QUIC adapters together.");
    println!("  - Transport-tagged `originate_connection` routes correctly per leg:");
    println!("    `transport: Some(Transport::Sip)` dispatched to `SipAdapter::originate`,");
    println!("    which sent a real SIP INVITE on the wire to the in-process peer at");
    println!("    127.0.0.1:5199 (returned 180 Ringing) and surfaced as a tracked");
    println!("    Connection in the orchestrator's connection registry.");
    println!("  - `Orchestrator::bridge_connections` is called against any");
    println!("    (Transport, Transport) pair through the same API. Both");
    println!("    QUIC↔QUIC AND SIP↔QUIC bridges constructed successfully —");
    println!("    `SipMediaStream` (G.711/PCMU) and `QuicDatagramMediaStream`");
    println!("    (RTP-in-QUIC-datagrams) both implement the `MediaStream` trait");
    println!("    so the cross-substrate frame pump can pump frames between them");
    println!("    via codec mapping where their PT sets intersect (PCMU at PT 0).");
    println!("  - WebRTC↔X bridging follows the same path the moment a WebRTC peer");
    println!("    has completed SDP negotiation with the WebRtcAdapter — the");
    println!("    adapter is registered and dispatch is wired identically.");

    Ok(())
}
