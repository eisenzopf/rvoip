//! Bounded, non-blocking UCTP media fanout.
//!
//! The network server registers each authorized subscriber's outbound media
//! queue with this publisher. The publisher itself is deliberately transport
//! neutral enough to be attached directly to an rvoip [`MediaGraph`].

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_core::broadcast::{
    BroadcastDescriptor, BroadcastDrainDescriptor, BroadcastDrainRequest, BroadcastDrainState,
    BroadcastHealthDescriptor, BroadcastHealthIssue, BroadcastHealthStatus,
    BroadcastLifecycleDescriptor, BroadcastLifecycleState, BroadcastProtocolDescriptor,
    BroadcastProtocolFamily, BroadcastPublisher, BroadcastSubstrate, BroadcastTransport,
};
use rvoip_core::capability::CodecInfo;
use rvoip_core::error::{Result, RvoipError};
use rvoip_core::stream::MediaFrame;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

pub const UCTP_QUIC_PROTOCOL_VERSION: &str = "uctp/0.2; rtp-datagram/1";

const PUBLISHER_READY: u8 = 0;
const PUBLISHER_DRAINING: u8 = 1;
const PUBLISHER_CLOSED: u8 = 2;

pub struct UctpBroadcastPublisher {
    session_id: String,
    stream_id: String,
    frame_tx: mpsc::Sender<MediaFrame>,
    subscribers: Arc<DashMap<u64, mpsc::Sender<MediaFrame>>>,
    next_subscriber: AtomicU64,
    max_subscribers: usize,
    lifecycle: AtomicU8,
    admission: parking_lot::Mutex<()>,
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
            lifecycle: AtomicU8::new(PUBLISHER_READY),
            admission: parking_lot::Mutex::new(()),
            task: task.abort_handle(),
        }))
    }

    /// Register a receive-only, already-authenticated network subscriber.
    pub fn add_subscriber(&self, target: mpsc::Sender<MediaFrame>) -> Result<u64> {
        let _admission = self.admission.lock();
        if self.lifecycle.load(Ordering::Acquire) != PUBLISHER_READY {
            return Err(RvoipError::InvalidState(
                "UCTP broadcast is draining or closed",
            ));
        }
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

    fn lifecycle_state(&self) -> BroadcastLifecycleState {
        match self.lifecycle.load(Ordering::Acquire) {
            PUBLISHER_DRAINING => BroadcastLifecycleState::Draining,
            PUBLISHER_CLOSED => BroadcastLifecycleState::Closed,
            _ => BroadcastLifecycleState::Ready,
        }
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

    fn protocol(&self) -> BroadcastProtocolDescriptor {
        BroadcastProtocolDescriptor {
            family: BroadcastProtocolFamily::Uctp,
            substrate: Some(BroadcastSubstrate::RawQuic),
            transport_version: "uctp/0.2".into(),
            media_format_version: None,
            object_format_version: None,
            media_profile: Some("rtp-datagram/1".into()),
        }
    }

    fn lifecycle(&self) -> BroadcastLifecycleDescriptor {
        BroadcastLifecycleDescriptor {
            state: self.lifecycle_state(),
            since: None,
        }
    }

    fn health(&self) -> BroadcastHealthDescriptor {
        let lifecycle = self.lifecycle_state();
        let subscribers = self.subscribers.len();
        let (status, issues) = match lifecycle {
            BroadcastLifecycleState::Closed => (BroadcastHealthStatus::Closed, Vec::new()),
            BroadcastLifecycleState::Draining => (
                BroadcastHealthStatus::Degraded,
                vec![BroadcastHealthIssue::Draining],
            ),
            _ if subscribers >= self.max_subscribers => (
                BroadcastHealthStatus::Degraded,
                vec![BroadcastHealthIssue::CapacityExhausted],
            ),
            _ => (BroadcastHealthStatus::Healthy, Vec::new()),
        };

        BroadcastHealthDescriptor {
            status,
            issues,
            active_subscribers: Some(subscribers.min(u32::MAX as usize) as u32),
            subscriber_capacity: Some(self.max_subscribers.min(u32::MAX as usize) as u32),
            checked_at: Utc::now(),
        }
    }

    async fn drain(
        self: Arc<Self>,
        request: BroadcastDrainRequest,
    ) -> Result<BroadcastDrainDescriptor> {
        let started_at = Utc::now();
        let previous = {
            let _admission = self.admission.lock();
            self.lifecycle.compare_exchange(
                PUBLISHER_READY,
                PUBLISHER_DRAINING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
        };
        if previous == Err(PUBLISHER_CLOSED) {
            return Ok(BroadcastDrainDescriptor {
                state: BroadcastDrainState::Drained,
                reason: request.reason,
                started_at,
                deadline: request.deadline,
                completed_at: Some(Utc::now()),
                remaining_subscribers: 0,
            });
        }

        while !self.subscribers.is_empty() && Utc::now() < request.deadline {
            self.subscribers.retain(|_, target| !target.is_closed());
            if !self.subscribers.is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
        let deadline_exceeded = !self.subscribers.is_empty();
        self.close().await?;

        Ok(BroadcastDrainDescriptor {
            state: if deadline_exceeded {
                BroadcastDrainState::DeadlineExceeded
            } else {
                BroadcastDrainState::Drained
            },
            reason: request.reason,
            started_at,
            deadline: request.deadline,
            completed_at: Some(Utc::now()),
            remaining_subscribers: 0,
        })
    }

    async fn close(self: Arc<Self>) -> Result<()> {
        let _admission = self.admission.lock();
        self.lifecycle.store(PUBLISHER_CLOSED, Ordering::Release);
        self.task.abort();
        self.subscribers.clear();
        metrics::gauge!("rvoip_uctp_broadcast_subscribers").set(0.0);
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

    #[tokio::test]
    async fn typed_status_rejects_new_subscribers_while_draining() {
        let publisher = UctpBroadcastPublisher::new("session", "audio", 10, 2).unwrap();
        let (subscriber_tx, subscriber_rx) = mpsc::channel(1);
        publisher.add_subscriber(subscriber_tx).unwrap();

        let drain = {
            let publisher = Arc::clone(&publisher);
            tokio::spawn(async move {
                publisher
                    .drain(BroadcastDrainRequest {
                        reason: rvoip_core::BroadcastDrainReason::Shutdown,
                        deadline: Utc::now() + chrono::Duration::seconds(1),
                    })
                    .await
            })
        };

        tokio::task::yield_now().await;
        assert_eq!(
            publisher.lifecycle().state,
            BroadcastLifecycleState::Draining
        );
        assert_eq!(
            publisher.health().issues,
            vec![BroadcastHealthIssue::Draining]
        );
        let (late_tx, _late_rx) = mpsc::channel(1);
        assert!(matches!(
            publisher.add_subscriber(late_tx),
            Err(RvoipError::InvalidState(_))
        ));

        drop(subscriber_rx);
        let drained = drain.await.unwrap().unwrap();
        assert_eq!(drained.state, BroadcastDrainState::Drained);
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Closed);
    }

    #[test]
    fn protocol_descriptor_separates_uctp_and_rtp_profile_versions() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let publisher = runtime
            .block_on(async { UctpBroadcastPublisher::new("session", "audio", 10, 2).unwrap() });
        assert_eq!(publisher.protocol().family, BroadcastProtocolFamily::Uctp);
        assert_eq!(
            publisher.protocol().substrate,
            Some(BroadcastSubstrate::RawQuic)
        );
        assert_eq!(publisher.protocol().transport_version, "uctp/0.2");
        assert_eq!(
            publisher.protocol().media_profile.as_deref(),
            Some("rtp-datagram/1")
        );
    }
}
