use std::error::Error;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use rvoip::app::{
    AppEvent, BridgeEvidence, Capability, CustomerPolicy, EmployeePolicy, HttpConfig, Role,
    RvoipApp, SipConfig, Transport, VoiceRoutingPolicy, WebRtcConfig,
};
use rvoip_core::capability::CodecInfo;
use rvoip_core::ids::{ConnectionId, StreamId};
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use rvoip_sip::{RegistrationHandle, UnifiedCoordinator};
use rvoip_webrtc::media::from_tracks;
use rvoip_webrtc::peer::{DataChannelOptions, PeerRole, RvoipDataChannel, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig as LowWebRtcConfig;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::warn;

type AnyError = Box<dyn Error + Send + Sync>;

const CUSTOMER_CHAT_LABEL: &str = "rvoip-chat";
const CALL_COMMAND: &str = "CALL_ALICE";
const ALICE_USER: &str = "alice";

#[derive(Parser, Debug)]
#[command(version, about = "Customer WebRTC chat escalates to SIP agent voice")]
struct Args {
    /// Static customer web page bind address.
    #[arg(long, default_value = "127.0.0.1:0")]
    http_bind: SocketAddr,
    /// WebRTC WebSocket signaling bind address.
    #[arg(long, default_value = "127.0.0.1:0")]
    ws_bind: SocketAddr,
    /// SIP listener/registrar bind address.
    #[arg(long, default_value = "127.0.0.1:0")]
    sip_bind: SocketAddr,
    /// SIP realm/AOR domain for Alice's registration.
    #[arg(long, default_value = "callcenter.local")]
    domain: String,
    /// Password accepted for sip:alice@<domain>.
    #[arg(long, default_value = "password123")]
    alice_password: String,
    /// Start in-process SIP/WebRTC clients and run bidirectional media proof.
    #[arg(long)]
    auto_proof: bool,
}

struct Gateway {
    app: RvoipApp,
    http_addr: SocketAddr,
    ws_addr: SocketAddr,
    sip_addr: SocketAddr,
    alice_aor: String,
    alice_password: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct WsSignal {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    sdp: String,
    #[serde(default, rename = "connection_id")]
    connection_id: String,
    #[serde(default)]
    candidate: String,
}

struct CustomerHarness {
    peer: Arc<RvoipPeerConnection>,
    chat: RvoipDataChannel,
    connection_id: ConnectionId,
}

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "info,rvoip_sip_dialog=warn,webrtc=warn".into()),
        )
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args = Args::parse();
    let gateway = start_gateway(&args).await?;
    print_startup_banner(&gateway);

    if args.auto_proof {
        run_auto_proof(&gateway).await?;
        return Ok(());
    }

    gateway.app.run().await?;
    println!("\n[shutdown] stopping customer escalation demo");
    Ok(())
}

async fn start_gateway(args: &Args) -> Result<Gateway, AnyError> {
    let app = build_call_center_app(args).await?;
    let addresses = app.addresses();
    let http_addr = addresses.http.ok_or("HTTP server did not start")?;
    let ws_addr = addresses
        .webrtc_ws
        .ok_or("WebRTC WS signaling did not start")?;
    let sip_addr = addresses.sip.ok_or("SIP server did not start")?;
    let alice_aor = format!("sip:{ALICE_USER}@{}", args.domain);

    Ok(Gateway {
        app,
        http_addr,
        ws_addr,
        sip_addr,
        alice_aor,
        alice_password: args.alice_password.clone(),
    })
}

async fn build_call_center_app(args: &Args) -> Result<RvoipApp, AnyError> {
    let static_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static");
    let app = RvoipApp::builder()
        .http(HttpConfig::bind(args.http_bind.to_string()).serve_static(static_root))
        .webrtc(
            WebRtcConfig::ws(args.ws_bind.to_string())
                .allow(Role::Customer, [Capability::Text, Capability::Voice])
                .allow(Role::Employee, [Capability::Text, Capability::Voice])
                .escalation_command(CALL_COMMAND),
        )
        .sip(
            SipConfig::bind(args.sip_bind.to_string())
                .domain(args.domain.clone())
                .allow(Role::Employee, [Capability::Voice])
                .registrar_users([(ALICE_USER, args.alice_password.as_str())]),
        )
        .employees(EmployeePolicy::named([ALICE_USER]))
        .customers(CustomerPolicy::webrtc_only())
        .assignment(rvoip::app::AssignmentPolicy::fixed(ALICE_USER))
        .voice_routing(VoiceRoutingPolicy::prefer([
            Transport::Sip,
            Transport::WebRtc,
            Transport::Uctp,
        ]))
        .on_message(|ctx, msg| async move {
            let reply = format!("Alice: I see your message: \"{}\"", msg.text);
            ctx.reply("Alice", reply).await
        })
        .build()
        .await?;
    Ok(app)
}

fn print_startup_banner(gateway: &Gateway) {
    println!();
    println!("=== Customer WebRTC Chat Escalates To SIP Agent Voice ===");
    println!("Customer page:       http://{}", gateway.http_addr);
    println!("WebRTC WS signaling: ws://{}", gateway.ws_addr);
    println!("SIP registrar:       sip:{}", gateway.sip_addr);
    println!("Alice AOR:           {}", gateway.alice_aor);
    println!("Alice auth user:     {ALICE_USER}");
    println!("Alice password:      {}", gateway.alice_password);
    println!();
    println!("Human demo:");
    println!(
        "  1. Register a SIP softphone as Alice to sip:{} with user `{}`.",
        gateway.sip_addr, ALICE_USER
    );
    println!("  2. Open http://{} in a browser.", gateway.http_addr);
    println!("  3. Send chat, click Call Alice, answer Alice's SIP phone.");
    println!();
}

async fn run_auto_proof(gateway: &Gateway) -> Result<(), AnyError> {
    println!("[auto] starting in-process SIP Alice and WebRTC customer");

    let (alice, _registration, mut alice_calls) = start_in_process_alice(
        gateway.sip_addr,
        &gateway.alice_aor,
        &gateway.alice_password,
    )
    .await?;

    let customer = connect_customer_via_ws(gateway.ws_addr).await?;
    println!(
        "[auto] customer WebRTC connected as {}",
        customer.connection_id
    );

    customer
        .chat
        .send_text("I need help with my account")
        .await?;
    let reply =
        RvoipPeerConnection::recv_data_channel_text(customer.chat.inner(), Duration::from_secs(5))
            .await?;
    println!("[auto] chat reply over DataChannel: {reply}");

    let mut app_events = gateway.app.subscribe_events();
    customer.chat.send_text(CALL_COMMAND).await?;
    let evidence = wait_for_call_established(&mut app_events).await?;
    let alice_session = tokio::time::timeout(Duration::from_secs(5), alice_calls.recv())
        .await
        .map_err(|_| "timed out waiting for Alice inbound SIP call")?
        .ok_or("Alice call receiver closed")?;

    run_media_proof(gateway, &alice, &alice_session, &customer, &evidence).await?;
    println!(
        "[auto] PASS: chat, registration resolution, SIP INVITE, bridge, and media proof succeeded"
    );
    Ok(())
}

async fn wait_for_call_established(
    events: &mut tokio::sync::broadcast::Receiver<AppEvent>,
) -> Result<BridgeEvidence, AnyError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(AppEvent::CallEstablished { evidence, .. })) => return Ok(evidence),
            Ok(Ok(AppEvent::CallFailed { reason, .. })) => {
                return Err(format!("voice escalation failed: {reason}").into());
            }
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                return Err("app event channel closed".into());
            }
            Err(_) => {}
        }
    }
    Err("timed out waiting for app CallEstablished event".into())
}

async fn start_in_process_alice(
    gateway_sip_addr: SocketAddr,
    alice_aor: &str,
    password: &str,
) -> Result<
    (
        Arc<UnifiedCoordinator>,
        RegistrationHandle,
        tokio::sync::mpsc::Receiver<rvoip_sip::SessionId>,
    ),
    AnyError,
> {
    let alice_addr = resolve_udp_bind_addr("127.0.0.1:0".parse()?)?;
    let alice = UnifiedCoordinator::new(rvoip_sip::Config::on(
        "alice-agent",
        alice_addr.ip(),
        alice_addr.port(),
    ))
    .await?;
    let registration = alice
        .register(format!("sip:{gateway_sip_addr}"), ALICE_USER, password)
        .with_from_uri(alice_aor.to_string())
        .with_expires(300)
        .send()
        .await?;

    for _ in 0..100 {
        if alice.is_registered(&registration).await.unwrap_or(false) {
            println!(
                "[auto] Alice registered from {} as {}",
                alice_addr, alice_aor
            );
            let calls = spawn_sip_auto_acceptor(Arc::clone(&alice));
            return Ok((alice, registration, calls));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err("Alice did not reach registered state".into())
}

fn spawn_sip_auto_acceptor(
    peer: Arc<UnifiedCoordinator>,
) -> tokio::sync::mpsc::Receiver<rvoip_sip::SessionId> {
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    tokio::spawn(async move {
        while let Some(info) = peer.get_incoming_call().await {
            let sid = info.session_id.clone();
            match peer.accept_call(&sid).await {
                Ok(()) => {
                    println!("[alice] auto-accepted inbound SIP call {}", sid.0);
                    let _ = tx.send(sid).await;
                }
                Err(error) => warn!(error = %error, "Alice failed to accept SIP call"),
            }
        }
    });
    rx
}

async fn connect_customer_via_ws(ws_addr: SocketAddr) -> Result<CustomerHarness, AnyError> {
    let mut config = LowWebRtcConfig::loopback();
    config.trickle_ice = false;
    let peer = RvoipPeerConnection::new(&config, PeerRole::Offerer).await?;
    let chat = peer
        .create_data_channel_typed(CUSTOMER_CHAT_LABEL, DataChannelOptions::reliable())
        .await?;
    let offer = peer.create_offer_and_gather().await?;

    let url = format!("ws://{ws_addr}");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await?;
    ws.send(WsMessage::Text(
        serde_json::to_string(&WsSignal {
            msg_type: "offer".into(),
            sdp: offer,
            connection_id: String::new(),
            candidate: String::new(),
        })?
        .into(),
    ))
    .await?;

    let (answer, connection_id) = loop {
        let Some(next) = ws.next().await else {
            return Err("WebRTC WS closed before answer".into());
        };
        let msg = next?;
        if !msg.is_text() {
            continue;
        }
        let signal: WsSignal = serde_json::from_str(msg.to_text()?)?;
        if signal.msg_type == "answer" {
            break (signal.sdp, ConnectionId::from_string(signal.connection_id));
        }
    };

    peer.set_remote_answer(&answer).await?;
    peer.wait_connected(Duration::from_secs(10)).await?;
    RvoipPeerConnection::wait_data_channel_open(chat.inner(), Duration::from_secs(10)).await?;
    let _ = ws.close(None).await;
    Ok(CustomerHarness {
        peer,
        chat,
        connection_id,
    })
}

async fn run_media_proof(
    gateway: &Gateway,
    alice: &Arc<UnifiedCoordinator>,
    alice_session: &rvoip_sip::SessionId,
    customer: &CustomerHarness,
    evidence: &BridgeEvidence,
) -> Result<(), AnyError> {
    if let Some(adapter) = gateway.app.webrtc_adapter() {
        if let Some(answerer_peer) = adapter
            .routes()
            .get(&evidence.customer_connection)
            .map(|route| route.peer.clone())
        {
            let _ = RvoipPeerConnection::prime_remote_track(
                &answerer_peer,
                &customer.peer,
                Duration::from_secs(3),
            )
            .await;
        }
    }

    let local_track = customer
        .peer
        .local_audio_track()
        .ok_or("customer WebRTC peer has no local audio track")?;
    let local_ssrc = customer
        .peer
        .local_audio_ssrc()
        .ok_or("customer WebRTC peer has no local audio SSRC")?;
    let codec = CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48000,
        channels: 2,
        fmtp: None,
    };
    let customer_stream = from_tracks(StreamId::new(), codec, local_track, local_ssrc, 111, None);

    verify_webrtc_to_sip(alice, alice_session, &customer_stream).await?;
    verify_sip_to_webrtc(alice, alice_session, &customer.peer, &customer_stream).await?;
    Ok(())
}

async fn verify_webrtc_to_sip(
    alice: &Arc<UnifiedCoordinator>,
    alice_session: &rvoip_sip::SessionId,
    customer_stream: &Arc<rvoip_webrtc::media::WebRtcMediaStream>,
) -> Result<(), AnyError> {
    let mut sub = alice.subscribe_to_audio(alice_session).await?;
    while sub.try_recv().is_ok() {}

    const TONE_HZ: f32 = 1100.0;
    let out = customer_stream.frames_out();
    let tone_frames = opus_tone_frames(TONE_HZ, 100);
    for (i, payload) in tone_frames.iter().enumerate() {
        let _ = out
            .send(MediaFrame {
                stream_id: customer_stream.id(),
                kind: StreamKind::Audio,
                payload: Bytes::from(payload.clone()),
                timestamp_rtp: (i as u32) * 960,
                captured_at: Utc::now(),
                payload_type: Some(111),
            })
            .await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut received_pcm = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while received_pcm.len() < 8000 * 2 && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), sub.recv()).await {
            Ok(Some(frame)) => received_pcm.extend(frame.samples),
            _ => break,
        }
    }
    let (ok, freq, rms_value) = assert_tone(&received_pcm, 8000, TONE_HZ, 80.0, 500.0);
    println!(
        "[media] WebRTC -> SIP: {} samples, observed {:.1} Hz, rms {:.0}, ok={}",
        received_pcm.len(),
        freq,
        rms_value,
        ok
    );
    if !ok {
        return Err("WebRTC -> SIP tone proof failed".into());
    }
    Ok(())
}

async fn verify_sip_to_webrtc(
    alice: &Arc<UnifiedCoordinator>,
    alice_session: &rvoip_sip::SessionId,
    customer_peer: &Arc<RvoipPeerConnection>,
    customer_stream: &Arc<rvoip_webrtc::media::WebRtcMediaStream>,
) -> Result<(), AnyError> {
    const TONE_HZ: f32 = 1760.0;

    let warmup = pcm_sine(TONE_HZ, 8000, 160 * 20, 12000);
    for (i, chunk) in warmup.chunks(160).enumerate() {
        if chunk.len() < 160 {
            break;
        }
        let audio =
            rvoip_media_core::types::AudioFrame::new(chunk.to_vec(), 8000, 1, (i as u32) * 160);
        alice.send_audio(alice_session, audio).await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let remote_track = customer_peer
        .wait_remote_track(Duration::from_secs(3))
        .await
        .ok_or("customer WebRTC peer did not receive remote SIP track")?;
    customer_stream.attach_remote(remote_track);
    let mut input = MediaStream::frames_in(customer_stream.as_ref());
    while input.try_recv().is_ok() {}

    let tone = pcm_sine(TONE_HZ, 8000, 160 * 100, 12000);
    for (i, chunk) in tone.chunks(160).enumerate() {
        if chunk.len() < 160 {
            break;
        }
        let audio = rvoip_media_core::types::AudioFrame::new(
            chunk.to_vec(),
            8000,
            1,
            ((20 + i) as u32) * 160,
        );
        alice.send_audio(alice_session, audio).await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut received = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while received.len() < 60 && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), input.recv()).await {
            Ok(Some(frame)) if !frame.payload.is_empty() => received.push(frame.payload.to_vec()),
            _ => break,
        }
    }
    let pcm = decode_opus_to_mono_pcm(&received);
    let (ok, freq, rms_value) = assert_tone(&pcm, 48000, TONE_HZ, 80.0, 500.0);
    println!(
        "[media] SIP -> WebRTC: {} Opus frames, observed {:.1} Hz, rms {:.0}, ok={}",
        received.len(),
        freq,
        rms_value,
        ok
    );
    if !ok {
        return Err("SIP -> WebRTC tone proof failed".into());
    }
    Ok(())
}

fn resolve_udp_bind_addr(addr: SocketAddr) -> Result<SocketAddr, AnyError> {
    if addr.port() != 0 {
        return Ok(addr);
    }
    let socket = std::net::UdpSocket::bind(addr)?;
    Ok(socket.local_addr()?)
}

fn pcm_sine(freq_hz: f32, sample_rate: u32, num_samples: usize, amplitude: i16) -> Vec<i16> {
    let two_pi_f = 2.0 * std::f32::consts::PI * freq_hz;
    let mut out = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let sample = (two_pi_f * t).sin() * amplitude as f32;
        out.push(sample as i16);
    }
    out
}

fn opus_tone_frames(freq_hz: f32, num_frames: usize) -> Vec<Vec<u8>> {
    use rvoip_media_core::codec::audio::common::AudioCodec;
    use rvoip_media_core::codec::audio::{OpusApplication, OpusCodec, OpusConfig};
    use rvoip_media_core::types::SampleRate;

    let mut encoder = OpusCodec::new(
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
    let mono = pcm_sine(freq_hz, 48000, 960 * num_frames, 12000);
    let stereo: Vec<i16> = mono.iter().flat_map(|sample| [*sample, *sample]).collect();
    let mut frames = Vec::with_capacity(num_frames);
    for chunk in stereo.chunks(1920) {
        if chunk.len() < 1920 {
            break;
        }
        let audio = rvoip_media_core::types::AudioFrame::new(chunk.to_vec(), 48000, 2, 0);
        frames.push(encoder.encode(&audio).expect("opus encode"));
    }
    frames
}

fn decode_opus_to_mono_pcm(payloads: &[Vec<u8>]) -> Vec<i16> {
    use rvoip_media_core::codec::audio::common::AudioCodec;
    use rvoip_media_core::codec::audio::{OpusApplication, OpusCodec, OpusConfig};
    use rvoip_media_core::types::SampleRate;

    let mut decoder = OpusCodec::new(
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
    for payload in payloads {
        if payload.is_empty() {
            continue;
        }
        if let Ok(audio) = decoder.decode(payload) {
            for pair in audio.samples.chunks(2) {
                let sample = if pair.len() == 2 {
                    ((pair[0] as i32 + pair[1] as i32) / 2) as i16
                } else {
                    pair[0]
                };
                mono.push(sample);
            }
        }
    }
    mono
}

fn dominant_freq_hz(samples: &[i16], sample_rate: u32) -> f32 {
    if samples.len() < 2 {
        return 0.0;
    }
    let mut crossings = 0usize;
    for window in samples.windows(2) {
        let (a, b) = (window[0], window[1]);
        if (a >= 0 && b < 0) || (a < 0 && b >= 0) {
            crossings += 1;
        }
    }
    let duration = samples.len() as f32 / sample_rate as f32;
    (crossings as f32 / 2.0) / duration
}

fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|sample| (*sample as f64).powi(2)).sum();
    (sum_sq / samples.len() as f64).sqrt() as f32
}

fn assert_tone(
    samples: &[i16],
    sample_rate: u32,
    expected_freq_hz: f32,
    tolerance_hz: f32,
    min_rms: f32,
) -> (bool, f32, f32) {
    let freq = dominant_freq_hz(samples, sample_rate);
    let energy = rms(samples);
    (
        (freq - expected_freq_hz).abs() < tolerance_hz && energy >= min_rms,
        freq,
        energy,
    )
}
