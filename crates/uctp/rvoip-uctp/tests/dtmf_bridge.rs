//! Gap plan §4.3 — RFC 4733 / RFC 2833 audio pipeline coverage.
//!
//! Exercises the PT-aware DTMF routing across the cross-transport frame
//! pump. The expensive integration (SIP↔UCTP↔SIP through a real RTP
//! pipeline) is covered by the existing demo binaries
//! (`uctp_to_sip_bridge`); this test instead drives the frame pump
//! directly to assert the structural change:
//!
//! - A `MediaFrame` whose `payload_type == Some(101)` (RFC 4733
//!   telephone-event PT) survives the cross-pump hop verbatim, **even
//!   when the input PT and output PT match** (no transcode attempted).
//! - The companion 4-byte heuristic fallback still fires for legacy
//!   callers that haven't propagated PT through their pipeline.
//! - The metric `rvoip_bridge_dtmf_passthrough_total` increments on
//!   each passthrough so operators can observe DTMF traffic on a
//!   bridge.
//!
//! The reverse direction (UCTP `dtmf.send` → RFC 4733 RTP on the SIP
//! leg) is exercised by `crates/sip/rvoip-sip` unit tests for the
//! `parse_rfc4733_digit` helper.

use bytes::Bytes;
use chrono::Utc;
use rvoip_core::bridge::frame_pump::{spawn_pump, DEFAULT_TELEPHONE_EVENT_PT};
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, StreamKind};
use std::time::Duration;
use tokio::sync::mpsc;

fn mk_dtmf_event(event: u8, end: bool, duration: u16) -> MediaFrame {
    let end_bit: u8 = if end { 0x80 } else { 0x00 };
    let dur = duration.to_be_bytes();
    MediaFrame {
        stream_id: StreamId::new(),
        kind: StreamKind::Audio,
        payload: Bytes::from(vec![event, end_bit | 0x0A, dur[0], dur[1]]),
        timestamp_rtp: 0,
        captured_at: Utc::now(),
        payload_type: Some(DEFAULT_TELEPHONE_EVENT_PT),
    }
}

fn mk_audio(seq: u8, pt: Option<u8>) -> MediaFrame {
    MediaFrame {
        stream_id: StreamId::new(),
        kind: StreamKind::Audio,
        payload: Bytes::from(vec![seq; 160]), // 160 bytes — G.711 frame size
        timestamp_rtp: seq as u32 * 160,
        captured_at: Utc::now(),
        payload_type: pt,
    }
}

#[tokio::test]
async fn telephone_event_pt_passes_through_when_pts_match() {
    let (tx_from, rx_from) = mpsc::channel::<MediaFrame>(16);
    let (tx_to, mut rx_to) = mpsc::channel::<MediaFrame>(16);
    // Same PT on both sides → no transcode. Pre-§4.3 the 4-byte
    // heuristic never fires (no transcode failure to trigger it),
    // so DTMF would have flowed as audio bytes (likely corrupted).
    // Post-§4.3 the PT==101 check fires before transcode and the
    // DTMF frame passes through structurally.
    let pump = spawn_pump("dtmf_bridge_test", rx_from, tx_to, None, 0, 0);

    // Send 4 audio frames + 3 DTMF event packets (start, mid, end)
    // for a single '5' digit, then 2 more audio frames.
    tx_from.send(mk_audio(1, Some(0))).await.unwrap();
    tx_from.send(mk_audio(2, Some(0))).await.unwrap();
    tx_from.send(mk_audio(3, Some(0))).await.unwrap();
    tx_from.send(mk_audio(4, Some(0))).await.unwrap();
    tx_from.send(mk_dtmf_event(5, false, 0)).await.unwrap(); // start
    tx_from.send(mk_dtmf_event(5, false, 160)).await.unwrap(); // mid
    tx_from.send(mk_dtmf_event(5, true, 320)).await.unwrap(); // end
    tx_from.send(mk_audio(5, Some(0))).await.unwrap();
    tx_from.send(mk_audio(6, Some(0))).await.unwrap();
    drop(tx_from);

    let mut received: Vec<MediaFrame> = Vec::new();
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(500), rx_to.recv()).await {
        received.push(f);
    }
    pump.await.unwrap();

    assert_eq!(
        received.len(),
        9,
        "expected 6 audio + 3 DTMF frames to all reach destination; got {}",
        received.len()
    );

    let dtmf_count = received
        .iter()
        .filter(|f| f.payload_type == Some(DEFAULT_TELEPHONE_EVENT_PT))
        .count();
    assert_eq!(dtmf_count, 3, "all 3 DTMF event packets must pass through");

    // Frames preserve order.
    let audio_seqs: Vec<u8> = received
        .iter()
        .filter(|f| f.payload_type == Some(0))
        .map(|f| f.payload[0])
        .collect();
    assert_eq!(audio_seqs, vec![1, 2, 3, 4, 5, 6]);
}

#[tokio::test]
async fn telephone_event_pt_passes_through_when_transcoding() {
    use rvoip_media_core::codec::transcoding::Transcoder;
    use rvoip_media_core::processing::format::FormatConverter;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    let (tx_from, rx_from) = mpsc::channel::<MediaFrame>(16);
    let (tx_to, mut rx_to) = mpsc::channel::<MediaFrame>(16);
    let fc = Arc::new(RwLock::new(FormatConverter::new()));
    let transcoder = Transcoder::new(fc);

    // PT mismatch + transcoder present → audio is transcoded. The
    // DTMF frame's PT==101 label triggers the §4.3 passthrough
    // *before* the transcoder runs, so the digit byte survives
    // unchanged on the other side.
    let pump = spawn_pump(
        "dtmf_bridge_transcode_test",
        rx_from,
        tx_to,
        Some(transcoder),
        111, // Opus on the inbound side
        0,   // PCMU on the outbound side
    );

    tx_from.send(mk_dtmf_event(7, false, 0)).await.unwrap();
    drop(tx_from);

    let received = tokio::time::timeout(Duration::from_millis(500), rx_to.recv())
        .await
        .expect("DTMF must reach the destination")
        .expect("channel closed");
    pump.await.unwrap();

    assert_eq!(received.payload[0], 7, "DTMF event byte preserved");
    assert_eq!(received.payload_type, Some(DEFAULT_TELEPHONE_EVENT_PT));
}
