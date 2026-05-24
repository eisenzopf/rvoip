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
                        // Plan C2 audio-pipeline: a transcode failure
                        // on a 4-byte payload is almost certainly an
                        // RFC 4733 telephone-event frame (DTMF) that
                        // doesn't decode as audio. Pass it through
                        // verbatim so SIP-side DTMF doesn't get
                        // silently dropped at the bridge boundary;
                        // the destination RTP receiver will route
                        // by its own PT when it sees the wire packet.
                        //
                        // Note: this is best-effort. Full per-frame
                        // payload-type routing requires plumbing the
                        // RTP PT into `MediaFrame`, which touches 70+
                        // construction sites and is deferred.
                        if frame.payload.len() == 4 {
                            metrics::counter!(
                                "rvoip_bridge_dtmf_passthrough_total",
                                "direction" => direction,
                            )
                            .increment(1);
                            trace!(
                                direction,
                                "rvoip-core::frame_pump: 4-byte transcode failure — likely RFC 4733 DTMF; passing through"
                            );
                            // Fall through to the `to.send(frame)` call
                            // below with the original payload.
                        } else {
                            warn!(
                                direction,
                                from_pt,
                                to_pt,
                                error = %e,
                                bytes = frame.payload.len(),
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

    /// C2 audio-pipeline: a 4-byte payload that fails to transcode
    /// (RFC 4733 telephone-event size) passes through to the
    /// destination instead of being dropped. Larger transcode
    /// failures still drop.
    #[tokio::test]
    async fn pump_passes_through_4byte_payload_when_transcode_fails() {
        use rvoip_media_core::codec::transcoding::Transcoder;
        use rvoip_media_core::processing::format::FormatConverter;
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let (tx_from, rx_from) = mpsc::channel::<MediaFrame>(8);
        let (tx_to, mut rx_to) = mpsc::channel::<MediaFrame>(8);
        let fc = Arc::new(RwLock::new(FormatConverter::new()));
        let transcoder = Transcoder::new(fc);

        // from=Opus (111), to=PCMU (0) — different PTs, so the pump
        // tries to transcode. Random 4-byte payload isn't a valid
        // Opus frame → transcode fails → DTMF-passthrough kicks in.
        let pump = spawn_pump("test_dtmf", rx_from, tx_to, Some(transcoder), 111, 0);

        let dtmf_like = MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0x01, 0x0F, 0x00, 0x50]), // 4 bytes, looks like a telephone-event
            timestamp_rtp: 0,
            captured_at: Utc::now(),
        };
        tx_from.send(dtmf_like.clone()).await.unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx_to.recv())
            .await
            .expect("dtmf passthrough must deliver the frame")
            .expect("channel still open");
        assert_eq!(
            received.payload.as_ref(),
            dtmf_like.payload.as_ref(),
            "4-byte DTMF-like payload must pass through unchanged"
        );

        drop(tx_from);
        pump.await.unwrap();
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
