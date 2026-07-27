//! Raw-audio framing and the rvoip media-stream boundary.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use chrono::Utc;
use rvoip_core::connection::Direction;
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{
    MediaFrame, MediaReceiverReservation, MediaStream, QualitySnapshot, StreamKind,
};
use rvoip_core::CodecInfo;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::{Result, VapiError};
use crate::types::VapiAudioFormat;

pub(crate) struct AudioFramer {
    frame_bytes: usize,
    buffer: BytesMut,
}

impl AudioFramer {
    pub(crate) fn new(format: VapiAudioFormat) -> Self {
        Self {
            frame_bytes: format.frame_bytes(),
            buffer: BytesMut::with_capacity(format.frame_bytes() * 2),
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub(crate) fn next_frame(&mut self) -> Option<Bytes> {
        (self.buffer.len() >= self.frame_bytes)
            .then(|| self.buffer.split_to(self.frame_bytes).freeze())
    }

    #[cfg(test)]
    pub(crate) fn buffered_bytes(&self) -> usize {
        self.buffer.len()
    }
}

pub(crate) struct VapiMediaStream {
    id: StreamId,
    format: VapiAudioFormat,
    incoming_tx: mpsc::Sender<MediaFrame>,
    incoming_rx: Arc<Mutex<Option<mpsc::Receiver<MediaFrame>>>>,
    outgoing_tx: mpsc::Sender<MediaFrame>,
    outgoing_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    active: AtomicBool,
    closed: AtomicBool,
    cancel: CancellationToken,
}

impl VapiMediaStream {
    pub(crate) fn new(
        format: VapiAudioFormat,
        incoming_capacity: usize,
        outgoing_capacity: usize,
        cancel: CancellationToken,
    ) -> Arc<Self> {
        let (incoming_tx, incoming_rx) = mpsc::channel(incoming_capacity);
        let (outgoing_tx, outgoing_rx) = mpsc::channel(outgoing_capacity);
        Arc::new(Self {
            id: StreamId::new(),
            format,
            incoming_tx,
            incoming_rx: Arc::new(Mutex::new(Some(incoming_rx))),
            outgoing_tx,
            outgoing_rx: Mutex::new(Some(outgoing_rx)),
            active: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            cancel,
        })
    }

    pub(crate) fn activate(&self) {
        self.active.store(true, Ordering::Release);
    }

    pub(crate) fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
    }

    pub(crate) fn take_outgoing_receiver(&self) -> Result<mpsc::Receiver<MediaFrame>> {
        self.outgoing_rx
            .lock()
            .map_err(|_| VapiError::NotActive)?
            .take()
            .ok_or(VapiError::NotActive)
    }

    pub(crate) fn try_push_incoming(&self, payload: Bytes, timestamp_rtp: u32) -> Result<()> {
        let frame = MediaFrame {
            stream_id: self.id.clone(),
            kind: StreamKind::Audio,
            payload,
            timestamp_rtp,
            captured_at: Utc::now(),
            payload_type: Some(self.format.payload_type()),
        };
        self.incoming_tx
            .try_send(frame)
            .map_err(|_| VapiError::MediaQueueOverflow)
    }

    pub(crate) fn incoming_pending_frames(&self) -> usize {
        self.incoming_tx
            .max_capacity()
            .saturating_sub(self.incoming_tx.capacity())
    }

    pub(crate) fn incoming_has_capacity(&self) -> bool {
        self.incoming_tx.capacity() > 0 && !self.incoming_tx.is_closed()
    }

    pub(crate) fn incoming_is_closed(&self) -> bool {
        self.incoming_tx.is_closed()
    }
}

#[async_trait]
impl MediaStream for VapiMediaStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }

    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }

    fn codec(&self) -> CodecInfo {
        self.format.codec()
    }

    fn direction(&self) -> Direction {
        Direction::Outbound
    }

    fn source_ready(&self) -> bool {
        self.active.load(Ordering::Acquire) && !self.closed.load(Ordering::Acquire)
    }

    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> RvoipResult<mpsc::Receiver<MediaFrame>> {
        Ok(self.reserve_frames_in()?.commit())
    }

    fn reserve_frames_in(&self) -> RvoipResult<MediaReceiverReservation> {
        if !self.source_ready() {
            return Err(RvoipError::InvalidState("Vapi media stream is not active"));
        }
        let receiver = self
            .incoming_rx
            .lock()
            .map_err(|_| RvoipError::InvalidState("Vapi media receiver lock is poisoned"))?
            .take()
            .ok_or(RvoipError::InvalidState(
                "Vapi media receiver has already been acquired",
            ))?;
        let slot = Arc::clone(&self.incoming_rx);
        Ok(MediaReceiverReservation::new(receiver, move |receiver| {
            let mut slot = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            debug_assert!(slot.is_none(), "reserved Vapi receiver slot was replaced");
            if slot.is_none() {
                *slot = Some(receiver);
            }
        }))
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.try_frames_out().unwrap_or_else(|_| mpsc::channel(1).0)
    }

    fn try_frames_out(&self) -> RvoipResult<mpsc::Sender<MediaFrame>> {
        if !self.source_ready() {
            return Err(RvoipError::InvalidState("Vapi media stream is not active"));
        }
        Ok(self.outgoing_tx.clone())
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.deactivate();
            self.cancel.cancel();
        }
        Ok(())
    }
}

pub(crate) fn append_bounded_frames(
    framer: &mut AudioFramer,
    payload: &[u8],
    queued: &mut VecDeque<Bytes>,
    max_frames: usize,
    max_payload_bytes: usize,
) -> Result<()> {
    if payload.len() > max_payload_bytes {
        return Err(VapiError::MediaQueueOverflow);
    }
    let buffered = framer
        .buffer
        .len()
        .checked_add(payload.len())
        .ok_or(VapiError::MediaQueueOverflow)?;
    let complete_frames = buffered / framer.frame_bytes;
    if complete_frames > max_frames.saturating_sub(queued.len()) {
        return Err(VapiError::MediaQueueOverflow);
    }
    framer.push(payload);
    while let Some(frame) = framer.next_frame() {
        if queued.len() >= max_frames {
            return Err(VapiError::MediaQueueOverflow);
        }
        queued.push_back(frame);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mulaw_framer_splits_and_coalesces_without_reordering() {
        let mut framer = AudioFramer::new(VapiAudioFormat::MuLaw8Khz);
        framer.push(&vec![1; 79]);
        assert!(framer.next_frame().is_none());
        framer.push(&vec![2; 82]);
        let first = framer.next_frame().unwrap();
        assert_eq!(first.len(), 160);
        assert!(first[..79].iter().all(|byte| *byte == 1));
        assert!(first[79..].iter().all(|byte| *byte == 2));
        assert_eq!(framer.buffered_bytes(), 1);
    }

    #[test]
    fn pcm_framer_emits_exact_twenty_millisecond_frames() {
        let mut framer = AudioFramer::new(VapiAudioFormat::PcmS16Le16Khz);
        framer.push(&vec![7; 1_280]);
        assert_eq!(framer.next_frame().unwrap().len(), 640);
        assert_eq!(framer.next_frame().unwrap().len(), 640);
        assert!(framer.next_frame().is_none());
    }

    #[test]
    fn bounded_outgoing_frames_fail_closed() {
        let mut framer = AudioFramer::new(VapiAudioFormat::MuLaw8Khz);
        let mut queued = VecDeque::new();
        assert_eq!(
            append_bounded_frames(&mut framer, &vec![0; 320], &mut queued, 1, 1024),
            Err(VapiError::MediaQueueOverflow)
        );
    }

    #[test]
    fn oversized_payload_is_rejected_before_buffering() {
        let mut framer = AudioFramer::new(VapiAudioFormat::MuLaw8Khz);
        let mut queued = VecDeque::new();
        assert_eq!(
            append_bounded_frames(&mut framer, &vec![0; 2048], &mut queued, 100, 1024),
            Err(VapiError::MediaQueueOverflow)
        );
        assert_eq!(framer.buffered_bytes(), 0);
        assert!(queued.is_empty());
    }
}
