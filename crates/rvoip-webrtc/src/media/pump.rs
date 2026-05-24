//! RTP frame pumps between webrtc-rs tracks and rvoip-core channels.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use rtc::statistics::report::RTCStatsReport;

use bytes::BytesMut;
use chrono::Utc;
use parking_lot::Mutex;
use rtc::rtp;
use rtc::shared::marshal::{Marshal, MarshalSize, Unmarshal};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};

use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, QualitySnapshot};

pub const FRAME_CHANNEL_CAP: usize = 64;

/// Inbound RTP statistics for [`QualitySnapshot`].
#[derive(Default)]
pub struct InboundStats {
    packets: AtomicU64,
    jitter_ms: Mutex<f32>,
    last_arrival: Mutex<Option<Instant>>,
    webrtc_jitter_ms: Mutex<f32>,
    webrtc_packet_loss_pct: Mutex<f32>,
}

impl InboundStats {
    fn record_packet(&self) {
        self.packets.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();
        let mut last = self.last_arrival.lock();
        if let Some(prev) = *last {
            let delta_ms = now.duration_since(prev).as_secs_f32() * 1000.0;
            // Exponential moving average of inter-arrival jitter.
            let mut jitter = self.jitter_ms.lock();
            *jitter = if *jitter == 0.0 {
                delta_ms.abs()
            } else {
                *jitter * 0.9 + delta_ms.abs() * 0.1
            };
        }
        *last = Some(now);
    }

    /// Merge the first inbound audio RTP stream from a webrtc-rs stats report.
    pub fn merge_webrtc_report(&self, report: &RTCStatsReport) {
        let Some(inbound) = report.inbound_rtp_streams().next() else {
            return;
        };
        let received = inbound.received_rtp_stream_stats.packets_received;
        let lost = inbound.received_rtp_stream_stats.packets_lost.max(0) as u64;
        let total = received.saturating_add(lost);
        let loss_pct = if total == 0 {
            0.0
        } else {
            (lost as f32 / total as f32) * 100.0
        };
        *self.webrtc_jitter_ms.lock() =
            (inbound.received_rtp_stream_stats.jitter * 1000.0) as f32;
        *self.webrtc_packet_loss_pct.lock() = loss_pct;
    }

    pub fn snapshot(&self) -> QualitySnapshot {
        let pump_jitter = *self.jitter_ms.lock();
        let webrtc_jitter = *self.webrtc_jitter_ms.lock();
        let webrtc_loss = *self.webrtc_packet_loss_pct.lock();
        QualitySnapshot {
            jitter_ms: if webrtc_jitter > 0.0 {
                webrtc_jitter
            } else {
                pump_jitter
            },
            packet_loss_pct: webrtc_loss,
            mos: None,
        }
    }
}

/// Spawn a task that reads RTP from a remote track into `frames_in`.
pub fn spawn_inbound_pump(
    track: Arc<dyn TrackRemote>,
    stream_id: StreamId,
    frames_in_tx: mpsc::Sender<MediaFrame>,
    stats: Arc<InboundStats>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Some(event) = track.poll().await else {
                tokio::task::yield_now().await;
                continue;
            };
            match event {
                TrackRemoteEvent::OnRtpPacket(pkt) => {
                    stats.record_packet();
                    let payload = rtp_packet_to_bytes(&pkt);
                    let frame = MediaFrame {
                        stream_id: stream_id.clone(),
                        kind: rvoip_core::stream::StreamKind::Audio,
                        payload,
                        timestamp_rtp: pkt.header.timestamp,
                        captured_at: Utc::now(),
                    };
                    if frames_in_tx.send(frame).await.is_err() {
                        break;
                    }
                }
                TrackRemoteEvent::OnEnded | TrackRemoteEvent::OnError => break,
                _ => {}
            }
        }
    })
}

/// Spawn a task that reads `frames_out` and writes RTP to a local track.
pub fn spawn_outbound_pump(
    track: Arc<TrackLocalStaticRTP>,
    mut frames_out_rx: mpsc::Receiver<MediaFrame>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(frame) = frames_out_rx.recv().await {
            let pkt = match bytes_to_rtp_packet(&frame.payload) {
                Ok(mut pkt) => {
                    if pkt.header.timestamp == 0 {
                        pkt.header.timestamp = frame.timestamp_rtp;
                    }
                    pkt
                }
                Err(_) => continue,
            };

            loop {
                match track.write_rtp(pkt.clone()).await {
                    Ok(()) => break,
                    Err(e) if e.to_string().contains("not binding") => {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                    Err(_) => return,
                }
            }
        }
    })
}

fn rtp_packet_to_bytes(pkt: &rtp::Packet) -> bytes::Bytes {
    let size = pkt.marshal_size();
    let mut buf = BytesMut::with_capacity(size);
    buf.resize(size, 0);
    if pkt.marshal_to(&mut buf).is_ok() {
        buf.freeze()
    } else {
        bytes::Bytes::new()
    }
}

fn bytes_to_rtp_packet(data: &bytes::Bytes) -> Result<rtp::Packet, rtc::shared::error::Error> {
    let mut buf = data.clone();
    let mut pkt = rtp::Packet::default();
    rtp::Packet::unmarshal(&mut buf)?;
    Ok(pkt)
}

/// Build a minimal silent Opus RTP packet for loopback tests.
pub fn silent_rtp_payload(seq: u16, timestamp: u32) -> bytes::Bytes {
    rtp_packet_to_bytes(&silent_rtp_packet(seq, timestamp))
}

/// Marshalled silent Opus RTP with an explicit SSRC.
pub fn silent_rtp_payload_for_ssrc(ssrc: u32, seq: u16, timestamp: u32) -> bytes::Bytes {
    rtp_packet_to_bytes(&silent_rtp_packet_for_ssrc(ssrc, seq, timestamp))
}

/// Parsed silent Opus RTP packet for direct `TrackLocal::write_rtp`.
pub fn silent_rtp_packet(seq: u16, timestamp: u32) -> rtp::Packet {
    silent_rtp_packet_for_ssrc(1, seq, timestamp)
}

/// Parsed silent Opus RTP packet with an explicit SSRC (must match the local track).
pub fn silent_rtp_packet_for_ssrc(ssrc: u32, seq: u16, timestamp: u32) -> rtp::Packet {
    rtp::Packet {
        header: rtp::Header {
            version: 2,
            padding: false,
            extension: false,
            marker: false,
            payload_type: 111,
            sequence_number: seq,
            timestamp,
            ssrc,
            ..Default::default()
        },
        payload: bytes::Bytes::from_static(&[0xF8, 0xFF, 0xFE]),
    }
}
