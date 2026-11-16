//! The rvoip media boundary: caller PCMU in, agent PCMU out.

use crate::{Result, WebexError};
use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use rvoip_core::{
    connection::Direction,
    error::{Result as CoreResult, RvoipError},
    ids::StreamId,
    peer_media::PeerMediaFrame,
    stream::{MediaFrame, MediaReceiverReservation, MediaStream, QualitySnapshot, StreamKind},
    CodecInfo,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Webex carries raw mono PCMU at 8 kHz in both directions.
pub(crate) const PCMU_PAYLOAD_TYPE: u8 = 0;

pub(crate) fn codec() -> CodecInfo {
    CodecInfo {
        name: "PCMU".into(),
        clock_rate_hz: 8_000,
        channels: 1,
        fmtp: None,
        payload_type: Some(PCMU_PAYLOAD_TYPE),
    }
}

pub(crate) struct AudioStream {
    id: StreamId,
    pub active: AtomicBool,
    pub cancel: CancellationToken,
    inbound: Mutex<Option<mpsc::Sender<MediaFrame>>>,
    receiver: Arc<Mutex<Option<mpsc::Receiver<MediaFrame>>>>,
    outbound: mpsc::Sender<MediaFrame>,
    peer: mpsc::Sender<PeerMediaFrame>,
    stall: Duration,
}

/// Agent audio routed to this connection, consumed only by its session task.
pub(crate) struct AudioReceivers {
    pub outbound: mpsc::Receiver<MediaFrame>,
    pub peer: mpsc::Receiver<PeerMediaFrame>,
}

impl AudioStream {
    pub fn new(
        capacity: usize,
        stall: Duration,
        cancel: CancellationToken,
    ) -> (Arc<Self>, AudioReceivers) {
        let (inbound, receiver) = mpsc::channel(capacity);
        let (outbound, out_rx) = mpsc::channel(capacity);
        let (peer, peer_rx) = mpsc::channel(capacity);
        (
            Arc::new(Self {
                id: StreamId::new(),
                active: AtomicBool::new(false),
                cancel,
                inbound: Mutex::new(Some(inbound)),
                receiver: Arc::new(Mutex::new(Some(receiver))),
                outbound,
                peer,
                stall,
            }),
            AudioReceivers {
                outbound: out_rx,
                peer: peer_rx,
            },
        )
    }

    pub fn stop(&self) {
        self.active.store(false, Ordering::Release);
        self.inbound.lock().take();
        self.cancel.cancel();
    }

    /// Queue caller audio toward the media graph. A full queue fails the
    /// call: caller speech is never dropped silently.
    pub fn input(&self, payload: Bytes, timestamp: u32) -> Result<()> {
        self.inbound
            .lock()
            .as_ref()
            .ok_or(WebexError::Closed)?
            .try_send(MediaFrame {
                stream_id: self.id.clone(),
                kind: StreamKind::Audio,
                payload,
                timestamp_rtp: timestamp,
                captured_at: chrono::Utc::now(),
                payload_type: Some(PCMU_PAYLOAD_TYPE),
            })
            .map_err(|_| WebexError::Capacity)
    }
}

#[async_trait]
impl MediaStream for AudioStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }
    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }
    fn codec(&self) -> CodecInfo {
        codec()
    }
    fn direction(&self) -> Direction {
        Direction::Inbound
    }
    fn source_ready(&self) -> bool {
        self.active.load(Ordering::Acquire) && !self.cancel.is_cancelled()
    }
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }
    fn try_frames_in(&self) -> CoreResult<mpsc::Receiver<MediaFrame>> {
        Ok(self.reserve_frames_in()?.commit())
    }
    fn reserve_frames_in(&self) -> CoreResult<MediaReceiverReservation> {
        if !self.source_ready() {
            return Err(RvoipError::InvalidState("Webex media is not ready"));
        }
        let rx = self
            .receiver
            .lock()
            .take()
            .ok_or(RvoipError::InvalidState("Webex media already reserved"))?;
        let slot = self.receiver.clone();
        Ok(MediaReceiverReservation::new(rx, move |rx| {
            *slot.lock() = Some(rx)
        }))
    }
    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.try_frames_out().unwrap_or_else(|_| mpsc::channel(1).0)
    }
    fn try_frames_out(&self) -> CoreResult<mpsc::Sender<MediaFrame>> {
        if !self.source_ready() {
            return Err(RvoipError::InvalidState("Webex media is not ready"));
        }
        Ok(self.outbound.clone())
    }
    fn try_peer_frames_out(&self) -> CoreResult<mpsc::Sender<PeerMediaFrame>> {
        if !self.source_ready() {
            return Err(RvoipError::InvalidState("Webex media is not ready"));
        }
        Ok(self.peer.clone())
    }
    /// Webex frames replies itself and paces playback, so its bridges must
    /// deliver every agent byte in order rather than drop under load.
    fn lossless_delivery_stall_timeout(&self) -> Option<Duration> {
        Some(self.stall)
    }
    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }
    fn has_quality_measurement(&self) -> bool {
        false
    }
    async fn close(self: Arc<Self>) -> CoreResult<()> {
        self.stop();
        Ok(())
    }
}
