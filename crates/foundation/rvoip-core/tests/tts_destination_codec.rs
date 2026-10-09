//! End-to-end proof that `Orchestrator::play_audio` delivers TTS audio in the
//! destination stream's negotiated codec: a provider that only produces PCM
//! (a known sine wave) must come out of the connection's outbound stream as
//! correctly labelled, correctly timestamped, real-time paced codec frames
//! that decode back to the same sine wave.

use bytes::Bytes;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    PlaybackOutcome, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::commands::{AudioSource, InboundAction};
use rvoip_core::config::Config;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::error::{Result as RvResult, RvoipError};
use rvoip_core::harness::{TtsAudioFormat, TtsPlayback, TtsProvider, TtsRequest};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, StreamId, TenantId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::playback::PcmPlaybackSource;
use rvoip_core::session::SessionMedium;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_media_core::codec::AudioCodecSpec;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

const SOURCE_RATE_HZ: u32 = 16_000;
const TONE_HZ: f64 = 440.0;
const AMPLITUDE: f64 = 10_000.0;
/// One second of source audio: exactly 50 frames of 20 ms.
const FRAMES: usize = 50;
/// Deliberately not a multiple of a 20 ms frame (320 samples at 16 kHz),
/// so the core must re-frame the provider's output.
const PROVIDER_CHUNK_SAMPLES: usize = 333;
/// Simulated synthesis latency per provider chunk. A pacer that sleeps
/// "20 ms after the previous send" accumulates this; a fixed schedule does
/// not.
const PROVIDER_LATENCY: Duration = Duration::from_millis(7);

fn sine(rate_hz: u32, samples: usize) -> Vec<i16> {
    (0..samples)
        .map(|n| {
            let t = n as f64 / f64::from(rate_hz);
            (AMPLITUDE * (2.0 * std::f64::consts::PI * TONE_HZ * t).sin()) as i16
        })
        .collect()
}

// --- Test adapter with a configurable negotiated codec ---------------------

struct TestStream {
    id: StreamId,
    codec: CodecInfo,
    inbound_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    outbound_tx: mpsc::Sender<MediaFrame>,
    outbound_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
}

#[async_trait::async_trait]
impl MediaStream for TestStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }
    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }
    fn codec(&self) -> CodecInfo {
        self.codec.clone()
    }
    fn direction(&self) -> Direction {
        Direction::Inbound
    }
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.inbound_rx
            .lock()
            .unwrap()
            .take()
            .expect("frames_in once")
    }
    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.outbound_tx.clone()
    }
    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot {
            jitter_ms: 0.0,
            packet_loss_pct: 0.0,
            mos: None,
            ..Default::default()
        }
    }
    async fn close(self: Arc<Self>) -> RvResult<()> {
        Ok(())
    }
}

struct OneStreamAdapter {
    inbound: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    stream: Arc<TestStream>,
}

#[async_trait::async_trait]
impl ConnectionAdapter for OneStreamAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    async fn originate(&self, _: OriginateRequest) -> RvResult<ConnectionHandle> {
        Err(RvoipError::NotImplemented("orig"))
    }
    async fn accept(&self, _: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn reject(&self, _: ConnectionId, _: RejectReason) -> RvResult<()> {
        Ok(())
    }
    async fn end(&self, _: ConnectionId, _: EndReason) -> RvResult<()> {
        Ok(())
    }
    async fn hold(&self, _: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn resume(&self, _: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn transfer(&self, _: ConnectionId, _: TransferTarget) -> RvResult<()> {
        Ok(())
    }
    async fn streams(&self, _: ConnectionId) -> RvResult<Vec<Arc<dyn MediaStream>>> {
        Ok(vec![self.stream.clone() as Arc<dyn MediaStream>])
    }
    async fn send_message(&self, _: ConnectionId, _: Message) -> RvResult<()> {
        Ok(())
    }
    async fn send_dtmf(&self, _: ConnectionId, _: &str, _: u32) -> RvResult<()> {
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _: ConnectionId,
        _: CapabilityDescriptor,
    ) -> RvResult<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.inbound.lock().unwrap().take().unwrap()
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }
    async fn verify_request_signature(
        &self,
        _: ConnectionId,
        _: SignatureHeaders,
    ) -> RvResult<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

async fn setup(codec: CodecInfo) -> (Arc<Orchestrator>, Arc<TestStream>, ConnectionId) {
    let orch = Orchestrator::new(Config::default());
    let (tx, rx) = mpsc::channel(16);
    let (_in_tx, in_rx) = mpsc::channel::<MediaFrame>(8);
    let (out_tx, out_rx) = mpsc::channel::<MediaFrame>(2 * FRAMES);
    let stream = Arc::new(TestStream {
        id: StreamId::new(),
        codec,
        inbound_rx: Mutex::new(Some(in_rx)),
        outbound_tx: out_tx,
        outbound_rx: Mutex::new(Some(out_rx)),
    });
    orch.register(Arc::new(OneStreamAdapter {
        inbound: Mutex::new(Some(rx)),
        stream: stream.clone(),
    }))
    .unwrap();
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let sid = orch
        .start_session(cid, SessionMedium::Voice, vec![])
        .await
        .unwrap();
    let connid = ConnectionId::new();
    tx.send(AdapterEvent::InboundConnection {
        connection: Connection {
            id: connid.clone(),
            session_id: sid.clone(),
            participant_id: ParticipantId::new(),
            transport: Transport::Sip,
            direction: Direction::Inbound,
            state: ConnectionState::Connecting,
            capabilities: CapabilityDescriptor::default(),
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: vec![],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        },
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.route_inbound_connection(
        connid.clone(),
        InboundAction::Accept {
            session_id: sid,
            participant_id: ParticipantId::new(),
        },
    )
    .await
    .unwrap();
    (orch, stream, connid)
}

// --- A TTS provider that only speaks PCM ----------------------------------

/// Emits a 16 kHz sine as little-endian PCM in odd-sized chunks, with
/// deliberately wrong stream identity, payload type and timestamps — the
/// core must not trust any of them.
struct SinePcmTts {
    request: Arc<Mutex<Option<TtsRequest>>>,
    cancelled: Arc<AtomicBool>,
}

struct SinePcmPlayback {
    remaining: Mutex<std::collections::VecDeque<i16>>,
    cancelled: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl TtsProvider for SinePcmTts {
    async fn synthesize(&self, request: TtsRequest) -> RvResult<Box<dyn TtsPlayback>> {
        *self.request.lock().unwrap() = Some(request);
        Ok(Box::new(SinePcmPlayback {
            remaining: Mutex::new(sine(SOURCE_RATE_HZ, FRAMES * 320).into()),
            cancelled: self.cancelled.clone(),
        }))
    }
}

#[async_trait::async_trait]
impl TtsPlayback for SinePcmPlayback {
    fn audio_format(&self) -> TtsAudioFormat {
        TtsAudioFormat::PcmS16Le {
            sample_rate_hz: SOURCE_RATE_HZ,
        }
    }
    async fn next_frame(&self) -> Option<MediaFrame> {
        tokio::time::sleep(PROVIDER_LATENCY).await;
        let chunk: Vec<i16> = {
            let mut remaining = self.remaining.lock().unwrap();
            let take = remaining.len().min(PROVIDER_CHUNK_SAMPLES);
            remaining.drain(..take).collect()
        };
        if chunk.is_empty() {
            return None;
        }
        Some(MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: chunk.iter().flat_map(|s| s.to_le_bytes()).collect(),
            timestamp_rtp: 12_345,
            captured_at: Utc::now(),
            payload_type: Some(120),
        })
    }
    async fn cancel(&self) -> RvResult<()> {
        self.cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn sine_tts() -> (SinePcmTts, Arc<Mutex<Option<TtsRequest>>>, Arc<AtomicBool>) {
    let request = Arc::new(Mutex::new(None));
    let cancelled = Arc::new(AtomicBool::new(false));
    (
        SinePcmTts {
            request: request.clone(),
            cancelled: cancelled.clone(),
        },
        request,
        cancelled,
    )
}

fn say(provider: &str) -> AudioSource {
    AudioSource::TtsRequest {
        provider_ref: provider.into(),
        text: "a tone, please".into(),
        voice: None,
    }
}

async fn collect(
    out: &mut mpsc::Receiver<MediaFrame>,
    frames: usize,
) -> Vec<(Instant, MediaFrame)> {
    let mut received = Vec::with_capacity(frames);
    while received.len() < frames {
        let frame = tokio::time::timeout(Duration::from_secs(5), out.recv())
            .await
            .expect("frame deadline")
            .expect("outbound stream closed");
        received.push((Instant::now(), frame));
    }
    received
}

/// Least-squares fit of the known tone (any phase/delay) to the decoded
/// steady-state signal. Returns (SINAD in dB, recovered amplitude).
fn tone_fit(decoded: &[f64], rate_hz: u32) -> (f64, f64) {
    let w = 2.0 * std::f64::consts::PI * TONE_HZ / f64::from(rate_hz);
    let (mut ss, mut cc, mut sc, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (n, y) in decoded.iter().enumerate() {
        let (s, c) = (w * n as f64).sin_cos();
        ss += s * s;
        cc += c * c;
        sc += s * c;
        ys += y * s;
        yc += y * c;
    }
    let det = ss * cc - sc * sc;
    let a = (ys * cc - yc * sc) / det;
    let b = (yc * ss - ys * sc) / det;
    let (mut signal, mut residual) = (0.0, 0.0);
    for (n, y) in decoded.iter().enumerate() {
        let (s, c) = (w * n as f64).sin_cos();
        let fit = a * s + b * c;
        signal += fit * fit;
        residual += (y - fit) * (y - fit);
    }
    (
        10.0 * (signal / residual.max(f64::MIN_POSITIVE)).log10(),
        (a * a + b * b).sqrt(),
    )
}

/// Everything the wire proves about one playback.
struct WireReport {
    stream_id_ok: bool,
    payload_types: Vec<Option<u8>>,
    timestamp_steps_ok: bool,
    pacing_ms: Vec<u128>,
    sinad_db: f64,
    amplitude: f64,
}

fn analyse(received: &[(Instant, MediaFrame)], stream: &TestStream, expected_pt: u8) -> WireReport {
    let codec = &stream.codec;
    let ticks = codec.clock_rate_hz / 50;
    let start = received[0].0;
    let mut decoder = AudioCodecSpec::new(
        &codec.name,
        expected_pt,
        codec.clock_rate_hz,
        codec.channels,
    )
    .build()
    .expect("decoder for the negotiated codec");
    let mut decoded = Vec::new();
    // Skip resampler/codec warm-up and the zero-padded final frame.
    for (index, (_, frame)) in received.iter().enumerate() {
        let audio = decoder.decode(&frame.payload).expect("decodable payload");
        let channels = usize::from(audio.channels.max(1));
        if (3..received.len() - 1).contains(&index) {
            decoded.extend(
                audio
                    .samples
                    .chunks(channels)
                    .map(|frame| f64::from(frame[0])),
            );
        }
    }
    let (sinad_db, amplitude) = tone_fit(&decoded, codec.clock_rate_hz);
    WireReport {
        stream_id_ok: received.iter().all(|(_, f)| f.stream_id == stream.id),
        payload_types: received.iter().map(|(_, f)| f.payload_type).collect(),
        timestamp_steps_ok: received.iter().enumerate().all(|(n, (_, f))| {
            f.timestamp_rtp.wrapping_sub(received[0].1.timestamp_rtp) == n as u32 * ticks
        }),
        pacing_ms: received
            .iter()
            .map(|(at, _)| at.duration_since(start).as_millis())
            .collect(),
        sinad_db,
        amplitude,
    }
}

async fn assert_pcm_tts_reaches_wire_in(codec: CodecInfo, expected_pt: u8, min_sinad_db: f64) {
    let (orch, stream, conn) = setup(codec.clone()).await;
    let (tts, request, cancelled) = sine_tts();
    orch.register_tts_provider("sine", Arc::new(tts));
    let mut out = stream.outbound_rx.lock().unwrap().take().unwrap();

    let handle = orch.play_audio(conn, say("sine")).await.unwrap();
    let received = collect(&mut out, FRAMES).await;
    assert_eq!(handle.wait().await.unwrap(), PlaybackOutcome::Completed);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), out.recv())
            .await
            .is_err(),
        "exactly {FRAMES} frames for one second of audio"
    );

    let report = analyse(&received, &stream, expected_pt);
    println!(
        "{} pt={expected_pt}: stream_id_ok={} timestamps_ok={} first_pts={:?} \
         arrival_ms[1,25,49]=[{},{},{}] sinad_db={:.1} amplitude={:.0}",
        codec.name,
        report.stream_id_ok,
        report.timestamp_steps_ok,
        &report.payload_types[..2],
        report.pacing_ms[1],
        report.pacing_ms[25],
        report.pacing_ms[49],
        report.sinad_db,
        report.amplitude,
    );

    let seen = request.lock().unwrap().take().expect("provider was asked");
    assert_eq!(seen.destination_codec.as_ref(), Some(&codec));
    assert_eq!(seen.sample_rate_hz, None);
    assert!(!cancelled.load(Ordering::SeqCst), "natural completion");

    assert!(
        report.stream_id_ok,
        "frames carry the destination stream id"
    );
    assert!(
        report
            .payload_types
            .iter()
            .all(|pt| *pt == Some(expected_pt)),
        "frames carry the negotiated payload type"
    );
    assert!(
        report.timestamp_steps_ok,
        "RTP timestamps advance by one 20 ms codec frame"
    );
    for (n, ms) in report.pacing_ms.iter().enumerate() {
        assert_eq!(*ms, n as u128 * 20, "frame {n} released on schedule");
    }
    assert!(
        report.sinad_db >= min_sinad_db,
        "decoded wire audio is the input tone: SINAD {:.1} dB",
        report.sinad_db
    );
    assert!(
        (report.amplitude - AMPLITUDE).abs() < AMPLITUDE * 0.1,
        "decoded tone keeps its level: {:.0}",
        report.amplitude
    );
}

#[tokio::test(start_paused = true)]
async fn pcm_tts_is_encoded_stamped_and_paced_for_pcmu() {
    assert_pcm_tts_reaches_wire_in(CodecInfo::from_name_with_defaults("PCMU"), 0, 30.0).await;
}

#[tokio::test(start_paused = true)]
async fn pcm_tts_is_encoded_stamped_and_paced_for_pcma() {
    let mut codec = CodecInfo::from_name_with_defaults("PCMA");
    codec.payload_type = Some(8);
    assert_pcm_tts_reaches_wire_in(codec, 8, 30.0).await;
}

#[cfg(feature = "opus")]
#[tokio::test(start_paused = true)]
async fn pcm_tts_is_encoded_stamped_and_paced_for_opus() {
    let codec = CodecInfo::from_name_with_defaults("opus").with_payload_type(111);
    assert_pcm_tts_reaches_wire_in(codec, 111, 25.0).await;
}

#[cfg(feature = "opus")]
#[tokio::test(start_paused = true)]
async fn pcm_tts_is_encoded_stamped_and_paced_for_stereo_opus() {
    let mut codec = CodecInfo::from_name_with_defaults("opus").with_payload_type(109);
    codec.channels = 2;
    assert_pcm_tts_reaches_wire_in(codec, 109, 25.0).await;
}

/// TTS prompts and caller-fed PCM share one encoder: identical input gives
/// byte-identical wire output.
async fn assert_tts_and_play_pcm_wire_output_match(codec: CodecInfo) {
    let (orch, stream, conn) = setup(codec).await;
    let (tts, _, _) = sine_tts();
    orch.register_tts_provider("sine", Arc::new(tts));
    let mut out = stream.outbound_rx.lock().unwrap().take().unwrap();

    let tts_playback = orch.play_audio(conn.clone(), say("sine")).await.unwrap();
    let from_tts = collect(&mut out, FRAMES).await;
    assert_eq!(
        tts_playback.wait().await.unwrap(),
        PlaybackOutcome::Completed
    );

    let (mut input, source) = PcmPlaybackSource::channel(SOURCE_RATE_HZ, 4).unwrap();
    let pcm_playback = orch.play_pcm(conn, source).await.unwrap();
    let feeder = tokio::spawn(async move {
        for chunk in sine(SOURCE_RATE_HZ, FRAMES * 320).chunks(320) {
            input.send(chunk).await.unwrap();
        }
        input.finish(&[]).await.unwrap();
    });
    let from_pcm = collect(&mut out, FRAMES).await;
    feeder.await.unwrap();
    assert_eq!(
        pcm_playback.wait().await.unwrap(),
        PlaybackOutcome::Completed
    );

    for ((_, tts), (_, pcm)) in from_tts.iter().zip(&from_pcm) {
        assert_eq!(tts.payload, pcm.payload);
        assert_eq!(tts.payload_type, pcm.payload_type);
        assert_eq!(tts.timestamp_rtp, pcm.timestamp_rtp);
        assert_eq!(tts.stream_id, pcm.stream_id);
    }
}

#[tokio::test(start_paused = true)]
async fn tts_and_play_pcm_produce_identical_pcmu_wire_output() {
    assert_tts_and_play_pcm_wire_output_match(CodecInfo::from_name_with_defaults("PCMU")).await;
}

#[cfg(feature = "opus")]
#[tokio::test(start_paused = true)]
async fn tts_and_play_pcm_produce_identical_opus_wire_output() {
    assert_tts_and_play_pcm_wire_output_match(
        CodecInfo::from_name_with_defaults("opus").with_payload_type(111),
    )
    .await;
}

// --- Providers that already speak the destination codec --------------------

struct EncodedTts {
    codec: CodecInfo,
    cancelled: Arc<AtomicBool>,
}

struct EncodedPlayback {
    codec: CodecInfo,
    remaining: Mutex<usize>,
    cancelled: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl TtsProvider for EncodedTts {
    async fn synthesize(&self, _: TtsRequest) -> RvResult<Box<dyn TtsPlayback>> {
        Ok(Box::new(EncodedPlayback {
            codec: self.codec.clone(),
            remaining: Mutex::new(FRAMES),
            cancelled: self.cancelled.clone(),
        }))
    }
}

#[async_trait::async_trait]
impl TtsPlayback for EncodedPlayback {
    fn audio_format(&self) -> TtsAudioFormat {
        TtsAudioFormat::Encoded {
            codec: self.codec.clone(),
        }
    }
    async fn next_frame(&self) -> Option<MediaFrame> {
        tokio::time::sleep(PROVIDER_LATENCY).await;
        let mut remaining = self.remaining.lock().unwrap();
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        Some(MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0xd5; 160]),
            timestamp_rtp: 999,
            captured_at: Utc::now(),
            payload_type: Some(96),
        })
    }
    async fn cancel(&self) -> RvResult<()> {
        self.cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn destination_encoded_tts_is_restamped_and_paced() {
    let mut codec = CodecInfo::from_name_with_defaults("PCMA");
    codec.payload_type = Some(8);
    let (orch, stream, conn) = setup(codec.clone()).await;
    orch.register_tts_provider(
        "encoded",
        Arc::new(EncodedTts {
            codec,
            cancelled: Arc::default(),
        }),
    );
    let mut out = stream.outbound_rx.lock().unwrap().take().unwrap();
    let handle = orch.play_audio(conn, say("encoded")).await.unwrap();
    let received = collect(&mut out, FRAMES).await;
    assert_eq!(handle.wait().await.unwrap(), PlaybackOutcome::Completed);
    let start = received[0].0;
    for (n, (at, frame)) in received.iter().enumerate() {
        assert_eq!(frame.stream_id, stream.id);
        assert_eq!(frame.payload_type, Some(8));
        assert_eq!(frame.timestamp_rtp, n as u32 * 160);
        assert_eq!(frame.payload.as_ref(), &[0xd5; 160]);
        assert_eq!(
            at.duration_since(start),
            Duration::from_millis(20) * n as u32
        );
    }
}

#[tokio::test(start_paused = true)]
async fn tts_encoded_in_another_codec_is_refused_and_cancelled() {
    let (orch, stream, conn) = setup(CodecInfo::from_name_with_defaults("PCMU")).await;
    let cancelled = Arc::new(AtomicBool::new(false));
    orch.register_tts_provider(
        "wrong",
        Arc::new(EncodedTts {
            codec: CodecInfo::from_name_with_defaults("opus"),
            cancelled: cancelled.clone(),
        }),
    );
    let mut out = stream.outbound_rx.lock().unwrap().take().unwrap();
    assert!(orch.play_audio(conn, say("wrong")).await.is_err());
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(cancelled.load(Ordering::SeqCst), "provider is cancelled");
    assert!(out.try_recv().is_err(), "nothing reaches the wire");
}
