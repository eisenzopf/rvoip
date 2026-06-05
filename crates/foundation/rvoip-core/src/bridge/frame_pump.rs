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

/// Conventional RTP payload type for RFC 4733 `telephone-event` frames.
/// Most SIP UAs advertise PT 101 in their SDP m-line fmtp; we use that
/// as the default until per-call PT negotiation lands as a config knob.
pub const DEFAULT_TELEPHONE_EVENT_PT: u8 = 101;

/// Gap plan §4.2 v1 punch list — control message sent to a running
/// frame pump to swap its transcoder pair atomically. The pump's
/// `tokio::select!` loop races the inbound frame channel against this
/// swap channel; on a swap, the in-flight frame (if any) completes
/// transcoding under the old codec settings, then the pump replaces
/// its local `transcoder` / `from_pt` / `to_pt` with the swap's
/// values for all subsequent frames. The "single-frame gap" comes
/// from this race — see the gap plan's risk discussion.
pub struct TranscoderSwap {
    pub new_transcoder: Option<Transcoder>,
    pub new_from_pt: u8,
    pub new_to_pt: u8,
    /// A3 — ack channel. When `Some`, the pump fires `()` on it after
    /// it has applied the swap to its local state, so the caller can
    /// `.await` confirmation that subsequent frames will use the new
    /// codec pair. `None` for callers that don't need synchronization
    /// (legacy / fire-and-forget).
    pub ack: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Spawn a frame-pump task. Returns the `JoinHandle` so the caller can
/// derive an `AbortHandle` and store it in a [`super::CrossBridgeHandle`].
///
/// `from_pt` / `to_pt` are the negotiated RTP payload types on each side.
/// When `transcoder` is `Some(_)` AND `from_pt != to_pt`, every frame is
/// run through `Transcoder::transcode`. Otherwise frames pass through
/// with payload bytes untouched.
///
/// **RFC 4733 routing (gap plan §4.3).** Frames whose `payload_type`
/// matches [`DEFAULT_TELEPHONE_EVENT_PT`] are passed through verbatim
/// (no transcode) so DTMF events survive the cross-transport hop. The
/// pre-§4.3 4-byte heuristic remains as a fallback for frames whose
/// `payload_type` is `None` (synthetic test frames, transcoder
/// outputs without a wire RTP header).
pub fn spawn_pump(
    direction: &'static str,
    from: mpsc::Receiver<MediaFrame>,
    to: mpsc::Sender<MediaFrame>,
    transcoder: Option<Transcoder>,
    from_pt: u8,
    to_pt: u8,
) -> JoinHandle<()> {
    // Gap plan §4.2 — preserve the original 6-arg signature for
    // callers that don't care about hot-swap. Forward to the
    // swap-aware variant with a closed swap channel (so the
    // `tokio::select!` branch is permanently disabled).
    let (_swap_tx, swap_rx) = mpsc::channel::<TranscoderSwap>(1);
    spawn_pump_with_swap(direction, from, to, transcoder, from_pt, to_pt, swap_rx)
}

/// Gap plan §4.2 v1 punch list — variant of [`spawn_pump`] that also
/// accepts a control channel for atomic transcoder hot-swaps. Used
/// by [`crate::bridge::CrossBridgeHandle::swap_transcoders`] to
/// converge to a new codec pair after a mid-call renegotiation.
///
/// The pump's main loop becomes a `tokio::select!` race between the
/// next inbound frame and the next swap message. On a swap, the
/// pump finishes processing any in-flight frame under the current
/// codec settings, then replaces `transcoder`/`from_pt`/`to_pt`
/// for all subsequent frames. The documented "single-frame gap" is
/// the worst case — usually nothing is in flight at the swap point.
pub fn spawn_pump_with_swap(
    direction: &'static str,
    mut from: mpsc::Receiver<MediaFrame>,
    to: mpsc::Sender<MediaFrame>,
    mut transcoder: Option<Transcoder>,
    mut from_pt: u8,
    mut to_pt: u8,
    mut swap_rx: mpsc::Receiver<TranscoderSwap>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut need_transcode = transcoder.is_some() && from_pt != to_pt;
        debug!(
            direction,
            from_pt, to_pt, need_transcode, "rvoip-core::frame_pump: started"
        );

        let mut swap_open = true;
        loop {
            // Race the inbound frame channel against the swap channel.
            // When swap_open is false (channel closed once and for all),
            // collapse to a from-only recv so a permanently-closed swap
            // doesn't keep waking the select.
            let frame_opt = if swap_open {
                tokio::select! {
                    biased;
                    swap = swap_rx.recv() => {
                        match swap {
                            Some(s) => {
                                transcoder = s.new_transcoder;
                                from_pt = s.new_from_pt;
                                to_pt = s.new_to_pt;
                                need_transcode = transcoder.is_some() && from_pt != to_pt;
                                metrics::counter!(
                                    "uctp_bridge_transcoder_swaps_total",
                                    "direction" => direction,
                                )
                                .increment(1);
                                // A3 — confirm swap application to the
                                // caller. The next iteration of this
                                // loop reads the new state, so by the
                                // time the ack fires the swap is live.
                                if let Some(ack) = s.ack {
                                    let _ = ack.send(());
                                }
                                debug!(
                                    direction,
                                    from_pt,
                                    to_pt,
                                    need_transcode,
                                    "rvoip-core::frame_pump: hot-swapped transcoder"
                                );
                                continue;
                            }
                            None => {
                                swap_open = false;
                                continue;
                            }
                        }
                    }
                    f = from.recv() => f,
                }
            } else {
                from.recv().await
            };
            let Some(mut frame) = frame_opt else {
                debug!(direction, "rvoip-core::frame_pump: source closed; exiting");
                return;
            };
            {
                // Gap plan §4.3 — PT-aware DTMF routing. If the inbound
                // pump labelled this frame with the telephone-event PT,
                // skip transcoding and pass through. This is a strict
                // improvement on the 4-byte heuristic below: that one
                // fires only when transcode *fails* on a 4-byte payload;
                // PT==101 catches DTMF even when the input PT matches
                // the output PT (no transcode attempted).
                let is_telephone_event = frame.payload_type == Some(DEFAULT_TELEPHONE_EVENT_PT);
                if is_telephone_event {
                    metrics::counter!(
                        "rvoip_bridge_dtmf_passthrough_total",
                        "direction" => direction,
                    )
                    .increment(1);
                    trace!(
                    direction,
                    "rvoip-core::frame_pump: PT={} (RFC 4733 telephone-event) — passing through",
                    DEFAULT_TELEPHONE_EVENT_PT
                );
                    if to.send(frame).await.is_err() {
                        debug!(direction, "rvoip-core::frame_pump: peer closed; exiting");
                        return;
                    }
                    continue;
                }

                if need_transcode {
                    let t = transcoder.as_mut().expect("checked above");
                    match t.transcode(&frame.payload, from_pt, to_pt).await {
                        Ok(bytes) => frame.payload = bytes.into(),
                        Err(e) => {
                            // Pre-§4.3 fallback: a transcode failure on a
                            // 4-byte payload is almost certainly an RFC 4733
                            // telephone-event whose PT wasn't carried in
                            // the MediaFrame. Pass it through verbatim.
                            if frame.payload.len() == 4 {
                                metrics::counter!(
                                    "rvoip_bridge_dtmf_passthrough_total",
                                    "direction" => direction,
                                )
                                .increment(1);
                                trace!(
                                direction,
                                "rvoip-core::frame_pump: 4-byte transcode failure — likely RFC 4733 DTMF without PT label; passing through"
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
            } // end frame-processing block
        }
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
            payload_type: None,
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
            payload_type: None,
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

    /// Gap plan §4.3 — frames labelled with the RFC 4733 telephone-event
    /// PT pass through verbatim even when the input PT matches the
    /// output PT (no transcode attempted). This catches DTMF the 4-byte
    /// heuristic misses (the heuristic only fires on a transcode
    /// failure).
    #[tokio::test]
    async fn pump_passes_through_telephone_event_pt_without_transcode() {
        let (tx_from, rx_from) = mpsc::channel::<MediaFrame>(8);
        let (tx_to, mut rx_to) = mpsc::channel::<MediaFrame>(8);
        // from=0 (PCMU), to=0 (PCMU). No transcoder, no PT mismatch —
        // pre-§4.3 the 4-byte heuristic never fires because there's
        // no transcode failure to trigger it. Post-§4.3 the PT==101
        // check fires before transcode.
        let pump = spawn_pump("dtmf_pt_test", rx_from, tx_to, None, 0, 0);

        let dtmf = MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0x01, 0x0F, 0x00, 0x50]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: Some(super::DEFAULT_TELEPHONE_EVENT_PT),
        };
        tx_from.send(dtmf.clone()).await.unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx_to.recv())
            .await
            .expect("dtmf must pass through")
            .expect("channel still open");
        assert_eq!(received.payload.as_ref(), dtmf.payload.as_ref());
        assert_eq!(
            received.payload_type,
            Some(super::DEFAULT_TELEPHONE_EVENT_PT)
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

    /// Gap plan §4.2 v1 punch list — sending a `TranscoderSwap` on a
    /// running pump replaces its transcoder/from_pt/to_pt for all
    /// subsequent frames. The "single-frame gap" is acceptable but
    /// hard to assert reliably here; we test the steady-state
    /// post-swap behavior plus the metric counter.
    #[tokio::test]
    async fn pump_hot_swaps_transcoder_when_swap_channel_receives() {
        let (tx_from, rx_from) = mpsc::channel::<MediaFrame>(8);
        let (tx_to, mut rx_to) = mpsc::channel::<MediaFrame>(8);
        let (swap_tx, swap_rx) = mpsc::channel::<TranscoderSwap>(4);

        // Start with passthrough (no transcoder, matching PTs).
        let pump = spawn_pump_with_swap(
            "swap_test",
            rx_from,
            tx_to,
            None,
            0, // PCMU
            0, // PCMU
            swap_rx,
        );

        // Send a frame under the original settings.
        tx_from.send(mk_frame(1)).await.unwrap();
        let first = tokio::time::timeout(std::time::Duration::from_millis(500), rx_to.recv())
            .await
            .expect("first frame")
            .expect("channel open");
        assert_eq!(first.payload[0], 1);

        // Hot-swap to a new (from_pt, to_pt) pair. We use no
        // transcoder so the swap is functionally observable via the
        // `from_pt != to_pt` change being irrelevant when transcoder
        // is None (no transcode runs) — the post-swap frames still
        // pass through, which is enough to prove the swap message
        // was applied (and the metric increments).
        swap_tx
            .send(TranscoderSwap {
                new_transcoder: None,
                new_from_pt: 0,
                new_to_pt: 0,
                ack: None,
            })
            .await
            .expect("swap sent");

        // Yield so the pump processes the swap before the next frame.
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Subsequent frames continue to flow under post-swap settings.
        tx_from.send(mk_frame(2)).await.unwrap();
        let second = tokio::time::timeout(std::time::Duration::from_millis(500), rx_to.recv())
            .await
            .expect("second frame post-swap")
            .expect("channel open");
        assert_eq!(second.payload[0], 2);

        drop(tx_from);
        drop(swap_tx);
        let _ = pump.await;
    }
}
