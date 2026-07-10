//! Bounded, non-blocking UCTP media fanout.
//!
//! The network server registers each authorized subscriber's outbound media
//! queue with this publisher. The publisher itself is deliberately transport
//! neutral enough to be attached directly to an rvoip [`MediaGraph`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use rvoip_core::broadcast::{BroadcastDescriptor, BroadcastPublisher, BroadcastTransport};
use rvoip_core::capability::CodecInfo;
use rvoip_core::error::{Result, RvoipError};
use rvoip_core::stream::MediaFrame;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

pub const UCTP_QUIC_PROTOCOL_VERSION: &str = "uctp/0.2; rtp-datagram/1";

pub struct UctpBroadcastPublisher {
    session_id: String,
    stream_id: String,
    frame_tx: mpsc::Sender<MediaFrame>,
    subscribers: Arc<DashMap<u64, mpsc::Sender<MediaFrame>>>,
    next_subscriber: AtomicU64,
    max_subscribers: usize,
    task: AbortHandle,
}

impl UctpBroadcastPublisher {
    pub fn new(
        session_id: impl Into<String>,
        stream_id: impl Into<String>,
        queue_frames: usize,
        max_subscribers: usize,
    ) -> Result<Arc<Self>> {
        if max_subscribers == 0 {
            return Err(RvoipError::AdmissionRejected(
                "UCTP subscriber limit is zero",
            ));
        }
        let (frame_tx, mut frame_rx) = mpsc::channel::<MediaFrame>(queue_frames.max(1));
        let subscribers = Arc::new(DashMap::<u64, mpsc::Sender<MediaFrame>>::new());
        let fanout = Arc::clone(&subscribers);
        let task = tokio::spawn(async move {
            while let Some(frame) = frame_rx.recv().await {
                let mut closed = Vec::new();
                for subscriber in fanout.iter() {
                    match subscriber.value().try_send(frame.clone()) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            metrics::counter!("rvoip_uctp_broadcast_dropped_frames_total")
                                .increment(1);
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            closed.push(*subscriber.key());
                        }
                    }
                }
                for id in closed {
                    fanout.remove(&id);
                }
            }
        });

        Ok(Arc::new(Self {
            session_id: session_id.into(),
            stream_id: stream_id.into(),
            frame_tx,
            subscribers,
            next_subscriber: AtomicU64::new(1),
            max_subscribers,
            task: task.abort_handle(),
        }))
    }

    /// Register a receive-only, already-authenticated network subscriber.
    pub fn add_subscriber(&self, target: mpsc::Sender<MediaFrame>) -> Result<u64> {
        if self.subscribers.len() >= self.max_subscribers {
            return Err(RvoipError::AdmissionRejected(
                "UCTP broadcast is at capacity",
            ));
        }
        let id = self.next_subscriber.fetch_add(1, Ordering::Relaxed);
        self.subscribers.insert(id, target);
        metrics::gauge!("rvoip_uctp_broadcast_subscribers").set(self.subscribers.len() as f64);
        Ok(id)
    }

    pub fn remove_subscriber(&self, id: u64) -> bool {
        let removed = self.subscribers.remove(&id).is_some();
        metrics::gauge!("rvoip_uctp_broadcast_subscribers").set(self.subscribers.len() as f64);
        removed
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }
}

#[async_trait]
impl BroadcastPublisher for UctpBroadcastPublisher {
    fn descriptor(&self) -> BroadcastDescriptor {
        BroadcastDescriptor {
            transport: BroadcastTransport::UctpQuic,
            namespace: self.session_id.clone(),
            audio_track: self.stream_id.clone(),
            catalog_track: None,
            protocol_version: UCTP_QUIC_PROTOCOL_VERSION.into(),
        }
    }

    fn codec(&self) -> CodecInfo {
        CodecInfo::from_name_with_defaults("opus")
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.frame_tx.clone()
    }

    async fn close(self: Arc<Self>) -> Result<()> {
        self.task.abort();
        self.subscribers.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use chrono::Utc;
    use rvoip_core::ids::StreamId;
    use rvoip_core::stream::MediaFrame;
    use rvoip_core::stream::StreamKind;

    use super::*;

    #[tokio::test]
    async fn slow_subscriber_does_not_block_others() {
        let publisher = UctpBroadcastPublisher::new("session", "audio", 10, 2).unwrap();
        let (slow_tx, _slow_rx) = mpsc::channel(1);
        let (fast_tx, mut fast_rx) = mpsc::channel(4);
        publisher.add_subscriber(slow_tx).unwrap();
        publisher.add_subscriber(fast_tx).unwrap();

        for sequence in 0..3 {
            publisher
                .frames_out()
                .send(MediaFrame {
                    stream_id: StreamId::new(),
                    kind: StreamKind::Audio,
                    payload: Bytes::from_static(b"opus"),
                    timestamp_rtp: sequence,
                    captured_at: Utc::now(),
                    payload_type: Some(111),
                })
                .await
                .unwrap();
        }
        for sequence in 0..3 {
            assert_eq!(fast_rx.recv().await.unwrap().timestamp_rtp, sequence);
        }
    }
}
