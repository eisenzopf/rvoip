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
            .expect("bridge source remains open");
    }

    fn take_output(&self) -> mpsc::Receiver<MediaFrame> {
        self.outbound_rx
            .lock()
            .expect("output receiver lock")
            .take()
            .expect("output receiver taken once")
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

async fn assert_g711_opus_round_trip(codec: &str, payload_type: u8, silence_byte: u8) {
    let sip = MockMediaStream::new(codec, 8_000);
    let connect = MockMediaStream::new("opus", 48_000);
    let mut sip_output = sip.take_output();
    let mut connect_output = connect.take_output();
    let bridge = bridge_streams(
        Arc::clone(&sip) as Arc<dyn MediaStream>,
        Arc::clone(&connect) as Arc<dyn MediaStream>,
    )
    .expect("create SIP↔Connect bridge");

    // One 20 ms G.711 frame (8 kHz × 20 ms = 160 samples).
    sip.inject(Bytes::from(vec![silence_byte; 160]), payload_type, 1_600)
        .await;
    let opus = tokio::time::timeout(Duration::from_secs(2), connect_output.recv())
        .await
        .expect("G.711→Opus media timeout")
        .expect("Connect sink remains open");
    assert_eq!(opus.payload_type, Some(111));
    assert!(!opus.payload.is_empty(), "Opus packet must contain audio");

    // Feed the production encoder's packet back through the reverse graph.
    connect.inject(opus.payload, 111, 9_600).await;
    let g711 = tokio::time::timeout(Duration::from_secs(2), sip_output.recv())
        .await
        .expect("Opus→G.711 media timeout")
        .expect("SIP sink remains open");
    assert_eq!(g711.payload_type, Some(payload_type));
    assert_eq!(g711.payload.len(), 160, "one 20 ms G.711 frame");

    bridge.stop();
}

#[tokio::test]
async fn pcmu_and_pcma_flow_bidirectionally_with_opus() {
    assert_g711_opus_round_trip("PCMU", 0, 0xff).await;
    assert_g711_opus_round_trip("PCMA", 8, 0xd5).await;
}
