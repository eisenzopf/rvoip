#![cfg(feature = "server")]

//! Credential-free media golden tests at the SIP↔Connect adapter boundary.
//! The mock streams use the same `MediaStream` channels as the real SIP and
//! WebRTC legs; `bridge_streams` uses the production MediaGraph/transcoder.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use rvoip_amazon_connect::bridge::bridge_streams;
use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_media_core::codec::audio::G711Codec;
use tokio::sync::mpsc;

struct MockMediaStream {
    id: StreamId,
    codec: CodecInfo,
    inbound_tx: mpsc::Sender<MediaFrame>,
    inbound_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    outbound_tx: mpsc::Sender<MediaFrame>,
    outbound_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
}

impl MockMediaStream {
    fn new(name: &str, clock_rate_hz: u32) -> Arc<Self> {
        let (inbound_tx, inbound_rx) = mpsc::channel(16);
        let (outbound_tx, outbound_rx) = mpsc::channel(16);
        Arc::new(Self {
            id: StreamId::new(),
            codec: CodecInfo {
                name: name.into(),
                clock_rate_hz,
                channels: 1,
                fmtp: None,
            },
            inbound_tx,
            inbound_rx: Mutex::new(Some(inbound_rx)),
            outbound_tx,
            outbound_rx: Mutex::new(Some(outbound_rx)),
        })
    }

    async fn inject(&self, payload: Bytes, payload_type: u8, timestamp_rtp: u32) {
        assert!(
            self.try_inject(payload, payload_type, timestamp_rtp).await,
            "bridge source remains open"
        );
    }

    async fn try_inject(&self, payload: Bytes, payload_type: u8, timestamp_rtp: u32) -> bool {
        self.inbound_tx
            .send(MediaFrame {
                stream_id: self.id.clone(),
                kind: StreamKind::Audio,
                payload,
                timestamp_rtp,
                captured_at: Utc::now(),
                payload_type: Some(payload_type),
            })
            .await
            .is_ok()
    }

    fn take_output(&self) -> mpsc::Receiver<MediaFrame> {
        self.outbound_rx
            .lock()
            .expect("output receiver lock")
            .take()
            .expect("output receiver taken once")
    }

    fn input_receiver_is_available(&self) -> bool {
        self.inbound_rx
            .lock()
            .expect("input receiver lock")
            .is_some()
    }
}

#[async_trait]
impl MediaStream for MockMediaStream {
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
            .expect("input receiver lock")
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.outbound_tx.clone()
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }

    async fn close(self: Arc<Self>) -> rvoip_core::error::Result<()> {
        Ok(())
    }
}

fn tone_samples(frame_count: usize) -> Vec<i16> {
    let sample_count = frame_count * 160;
    (0..sample_count)
        .map(|sample| {
            let phase = 2.0 * std::f32::consts::PI * 440.0 * sample as f32 / 8_000.0;
            (phase.sin() * 10_000.0) as i16
        })
        .collect()
}

fn rms(samples: &[i16]) -> f64 {
    let energy = samples
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / samples.len().max(1) as f64;
    energy.sqrt()
}

fn zero_crossings(samples: &[i16]) -> usize {
    samples
        .windows(2)
        .filter(|pair| (pair[0] < 0 && pair[1] >= 0) || (pair[0] >= 0 && pair[1] < 0))
        .count()
}

async fn assert_g711_opus_round_trip(codec: &str, payload_type: u8) {
    const FRAMES: usize = 10;
    let sip = MockMediaStream::new(codec, 8_000);
    let connect = MockMediaStream::new("opus", 48_000);
    let mut sip_output = sip.take_output();
    let mut connect_output = connect.take_output();
    let bridge = bridge_streams(
        Arc::clone(&sip) as Arc<dyn MediaStream>,
        Arc::clone(&connect) as Arc<dyn MediaStream>,
    )
    .expect("create SIP↔Connect bridge");

    let tone = tone_samples(FRAMES);
    let mut g711_codec = if payload_type == 0 {
        G711Codec::mu_law(8_000, 1).expect("PCMU codec")
    } else {
        G711Codec::a_law(8_000, 1).expect("PCMA codec")
    };
    let mut encoded_frames = Vec::with_capacity(FRAMES);
    for (index, samples) in tone.chunks_exact(160).enumerate() {
        let mut encoded = vec![0; 160];
        let written = g711_codec
            .encode_to_buffer(samples, &mut encoded)
            .expect("encode deterministic G.711 tone");
        encoded.truncate(written);
        encoded_frames.push(encoded.clone());
        sip.inject(
            Bytes::from(encoded),
            payload_type,
            8_000 + index as u32 * 160,
        )
        .await;
    }

    let mut opus_frames = Vec::with_capacity(FRAMES);
    for index in 0..FRAMES {
        let opus = tokio::time::timeout(Duration::from_secs(2), connect_output.recv())
            .await
            .expect("G.711→Opus media timeout")
            .expect("Connect sink remains open");
        assert_eq!(opus.payload_type, Some(111));
        assert!(!opus.payload.is_empty(), "Opus packet must contain audio");
        assert_eq!(
            opus.timestamp_rtp,
            8_000 + index as u32 * 960,
            "Opus clock advances 960 ticks per 20 ms"
        );
        opus_frames.push(opus);
    }

    // Feed production Opus packets back through the reverse graph and verify
    // the 48 kHz clock is translated back to 8 kHz.
    for opus in opus_frames {
        connect.inject(opus.payload, 111, opus.timestamp_rtp).await;
    }
    let mut decoded = Vec::with_capacity(FRAMES * 160);
    for index in 0..FRAMES {
        let g711 = tokio::time::timeout(Duration::from_secs(2), sip_output.recv())
            .await
            .expect("Opus→G.711 media timeout")
            .expect("SIP sink remains open");
        assert_eq!(g711.payload_type, Some(payload_type));
        assert_eq!(g711.payload.len(), 160, "one 20 ms G.711 frame");
        assert_eq!(
            g711.timestamp_rtp,
            8_000 + index as u32 * 160,
            "G.711 clock advances 160 ticks per 20 ms"
        );
        let mut pcm = vec![0_i16; 160];
        let samples = g711_codec
            .decode_to_buffer(&g711.payload, &mut pcm)
            .expect("decode bridged G.711 tone");
        decoded.extend_from_slice(&pcm[..samples]);
    }

    // Ignore two frames of codec warm-up. The remaining signal must preserve
    // meaningful energy and the 440 Hz zero-crossing fingerprint.
    let input = &tone[320..];
    let output = &decoded[320..];
    let energy_ratio = rms(output) / rms(input);
    assert!(
        (0.35..=1.65).contains(&energy_ratio),
        "decoded tone energy ratio {energy_ratio:.3} outside codec tolerance"
    );
    let input_crossings = zero_crossings(input);
    let output_crossings = zero_crossings(output);
    let crossing_error = input_crossings.abs_diff(output_crossings);
    assert!(
        crossing_error <= input_crossings / 5 + 2,
        "tone fingerprint changed: input crossings={input_crossings}, output={output_crossings}"
    );

    let post_teardown_payload = encoded_frames[0].clone();
    bridge.stop();
    tokio::task::yield_now().await;
    let _ = sip
        .try_inject(Bytes::from(post_teardown_payload), payload_type, 99_000)
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), connect_output.recv())
            .await
            .is_err(),
        "no media may be delivered after bridge teardown"
    );
}

#[tokio::test]
async fn pcmu_and_pcma_flow_bidirectionally_with_opus() {
    assert_g711_opus_round_trip("PCMU", 0).await;
    assert_g711_opus_round_trip("PCMA", 8).await;
}

#[test]
fn unsupported_codec_is_rejected_before_either_receiver_is_taken() {
    let supported = MockMediaStream::new("pcmu", 8_000);
    let unsupported = MockMediaStream::new("not-a-real-codec", 8_000);

    assert!(bridge_streams(
        Arc::clone(&supported) as Arc<dyn MediaStream>,
        Arc::clone(&unsupported) as Arc<dyn MediaStream>,
    )
    .is_err());
    assert!(supported.input_receiver_is_available());
    assert!(unsupported.input_receiver_is_available());
}

#[tokio::test]
async fn bridge_retains_stream_owners_until_teardown() {
    let sip = MockMediaStream::new("pcmu", 8_000);
    let connect = MockMediaStream::new("opus", 48_000);
    let sip_weak = Arc::downgrade(&sip);
    let connect_weak = Arc::downgrade(&connect);

    let bridge = bridge_streams(
        Arc::clone(&sip) as Arc<dyn MediaStream>,
        Arc::clone(&connect) as Arc<dyn MediaStream>,
    )
    .expect("create retained bridge");
    drop(sip);
    drop(connect);

    assert!(sip_weak.upgrade().is_some());
    assert!(connect_weak.upgrade().is_some());
    bridge.stop();
    assert!(sip_weak.upgrade().is_none());
    assert!(connect_weak.upgrade().is_none());
}
