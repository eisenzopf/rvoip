//! Per-direction frame pump task.
//!
//! Reads `MediaFrame`s from one `MediaStream`'s `frames_in()`, optionally
//! transcodes via `rvoip_media_core::Transcoder` when the RTP payload
//! type changes (codec mismatch), and writes to the peer's
//! `frames_out()`. One pump task per direction — bidirectional bridges
//! spawn two.
//!
//! Each pump **owns** its own `Transcoder` (rather than sharing through a
//! lock) because `Transcoder` contains `dyn AudioCodec` which is not
//! `Sync`. Per-direction ownership eliminates locking overhead and the
//! Send-across-await trap; the memory cost is one extra Transcoder per
//! bridge.
//!
//! Exits cleanly when:
//! - The source channel closes (peer hung up).
//! - The destination channel closes (we hung up; `send` returns Err).
//! - The task is aborted via the [`super::CrossBridgeHandle`] abort handle
//!   (i.e. `unbridge_connections` was called).

use rvoip_media_core::codec::transcoding::Transcoder;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, trace, warn};

use crate::stream::MediaFrame;

/// Spawn a frame-pump task. Returns the `JoinHandle` so the caller can
/// derive an `AbortHandle` and store it in a [`super::CrossBridgeHandle`].
///
/// `from_pt` / `to_pt` are the negotiated RTP payload types on each side.
/// When `transcoder` is `Some(_)` AND `from_pt != to_pt`, every frame is
/// run through `Transcoder::transcode`. Otherwise frames pass through
/// with payload bytes untouched.
pub fn spawn_pump(
    direction: &'static str,
    mut from: mpsc::Receiver<MediaFrame>,
    to: mpsc::Sender<MediaFrame>,
    mut transcoder: Option<Transcoder>,
    from_pt: u8,
    to_pt: u8,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let need_transcode = transcoder.is_some() && from_pt != to_pt;
        debug!(
            direction,
            from_pt,
            to_pt,
            need_transcode,
            "rvoip-core::frame_pump: started"
        );

        while let Some(mut frame) = from.recv().await {
            if need_transcode {
                let t = transcoder.as_mut().expect("checked above");
                match t.transcode(&frame.payload, from_pt, to_pt).await {
                    Ok(bytes) => frame.payload = bytes.into(),
                    Err(e) => {
                        warn!(
                            direction,
                            from_pt,
                            to_pt,
                            error = %e,
                            "rvoip-core::frame_pump: transcode failed; dropping frame"
                        );
                        metrics::counter!(
                            "rvoip_bridge_transcode_errors_total",
                            "direction" => direction,
                        )
                        .increment(1);
                        continue;
                    }
                }
            }
            trace!(direction, bytes = frame.payload.len(), "frame");
            if to.send(frame).await.is_err() {
                debug!(direction, "rvoip-core::frame_pump: peer closed; exiting");
                return;
            }
        }
        debug!(direction, "rvoip-core::frame_pump: source closed; exiting");
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::StreamId;
    use crate::stream::StreamKind;
    use bytes::Bytes;
    use chrono::Utc;

    fn mk_frame(seq: u8) -> MediaFrame {
        MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![seq]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
        }
    }

    /// Same payload-type on both sides → frames pass through bytes-identical.
    #[tokio::test]
    async fn pump_passes_through_when_pts_match() {
        let (tx_from, rx_from) = mpsc::channel::<MediaFrame>(8);
        let (tx_to, mut rx_to) = mpsc::channel::<MediaFrame>(8);
        let pump = spawn_pump("test", rx_from, tx_to, None, 111, 111);

        for i in 0u8..5 {
            tx_from.send(mk_frame(i)).await.unwrap();
        }
        drop(tx_from);

        let mut received = Vec::new();
        while let Some(f) = rx_to.recv().await {
            received.push(f.payload[0]);
        }
        assert_eq!(received, (0u8..5).collect::<Vec<_>>());

        pump.await.unwrap();
    }

    /// Aborting the pump cancels the task. Verify via the join handle's
    /// terminal state (Cancelled) rather than checking that no further
    /// frames arrive — the latter is racy because mpsc channel buffer
    /// + tokio scheduler ordering allow one queued frame to slip
    /// through before the abort takes effect at the next await point.
    #[tokio::test]
    async fn pump_aborts_cleanly() {
        let (tx_from, rx_from) = mpsc::channel::<MediaFrame>(8);
        let (tx_to, _rx_to) = mpsc::channel::<MediaFrame>(8);
        let pump = spawn_pump("test", rx_from, tx_to, None, 111, 111);
        let abort = pump.abort_handle();

        tx_from.send(mk_frame(0)).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        abort.abort();
        let outcome = pump.await;
        assert!(
            matches!(&outcome, Err(e) if e.is_cancelled()),
            "expected cancelled task; got {:?}",
            outcome.as_ref().map(|_| "ok").unwrap_or("err")
        );
    }

    /// When destination closes, the pump exits without panic.
    #[tokio::test]
    async fn pump_exits_when_destination_closes() {
        let (tx_from, rx_from) = mpsc::channel::<MediaFrame>(8);
        let (tx_to, rx_to) = mpsc::channel::<MediaFrame>(8);
        drop(rx_to); // close destination
        let pump = spawn_pump("test", rx_from, tx_to, None, 111, 111);
        tx_from.send(mk_frame(0)).await.unwrap();
        // The pump tries to send to a closed channel → exits.
        tokio::time::timeout(std::time::Duration::from_secs(2), pump)
            .await
            .expect("pump should exit on closed dest")
            .unwrap();
    }
}
