//! Generation-aware transport queue entries.
use crate::peer_switch::{PeerDeliveryGuard, PeerRouteTicket};
use crate::stream::MediaFrame;

/// Carries the speaking generation through buffering to the transport pump.
/// Queue admission alone is not delivery admission.
pub struct PeerMediaFrame {
    frame: MediaFrame,
    ticket: PeerRouteTicket,
}

impl PeerMediaFrame {
    pub fn new(frame: MediaFrame, ticket: PeerRouteTicket) -> Self {
        Self { frame, ticket }
    }

    /// Called by the transport after dequeueing. Stale entries are discarded.
    /// Keep the returned guard alive until the local send completes; moving
    /// work into a detached task requires moving the guard with it as well.
    pub fn into_delivery(self) -> Option<(MediaFrame, PeerDeliveryGuard)> {
        let guard = self.ticket.try_begin_delivery()?;
        Some((self.frame, guard))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ids::StreamId, stream::StreamKind};

    fn frame() -> MediaFrame {
        MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: bytes::Bytes::from_static(b"audio"),
            timestamp_rtp: 0,
            captured_at: chrono::Utc::now(),
            payload_type: Some(0),
        }
    }

    #[tokio::test]
    async fn queued_old_frame_is_rejected_after_commit() {
        let old = PeerRouteTicket::initial();
        let next = old.stage().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tx.send(PeerMediaFrame::new(frame(), old.clone()))
            .await
            .unwrap();
        assert!(next.commit_from(&old));
        assert!(rx.recv().await.unwrap().into_delivery().is_none());
    }

    #[test]
    fn dequeued_frame_blocks_commit_until_its_send_guard_is_released() {
        let old = PeerRouteTicket::initial();
        let next = old.stage().unwrap();
        let (frame, sending) = PeerMediaFrame::new(frame(), old.clone())
            .into_delivery()
            .unwrap();
        assert_eq!(frame.payload.as_ref(), b"audio");
        assert!(!next.commit_from(&old));
        drop(sending);
        assert!(next.commit_from(&old));
    }
}
