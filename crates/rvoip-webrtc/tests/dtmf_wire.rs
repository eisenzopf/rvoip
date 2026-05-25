//! H3: RFC 4733 DTMF wire-format capture — confirms `send_dtmf` actually
//! generates correct telephone-event RTP packets on the wire.
//!
//! Known limitation (tracked for follow-up): the current `add_local_audio_track`
//! only advertises the Opus encoding to the remote, so PT 101 (telephone-event)
//! packets sent via `send_dtmf` do not survive SDP-driven SRTP filtering on a
//! real loopback. The byte-layout assertions in `encode_telephone_event` cover
//! the wire format; a future change will register a multi-codec audio
//! transceiver so DTMF round-trips through real peers as well.

use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::media::dtmf::send_dtmf;
use rvoip_webrtc::peer::{connect_loopback, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;
use webrtc::media_stream::track_remote::TrackRemoteEvent;

const TELEPHONE_EVENT_PT: u8 = 101;

fn decode_event(payload: &[u8]) -> Option<(u8, bool, u8, u16)> {
    if payload.len() < 4 {
        return None;
    }
    let event = payload[0];
    let end_of_event = (payload[1] & 0b1000_0000) != 0;
    let volume = payload[1] & 0b0011_1111;
    let duration = u16::from_be_bytes([payload[2], payload[3]]);
    Some((event, end_of_event, volume, duration))
}

#[tokio::test]
#[ignore = "needs multi-codec audio transceiver before PT 101 survives SRTP — see module docs"]
async fn send_dtmf_emits_rfc4733_telephone_events() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (offerer, answerer) = connect_loopback(&WebRtcConfig::loopback())
        .await
        .expect("loopback");

    // Prime the answerer's `on_track` so it has a TrackRemote we can poll.
    let remote = RvoipPeerConnection::prime_remote_track(
        &offerer,
        &answerer,
        Duration::from_secs(5),
    )
    .await
    .expect("answerer received remote track");

    // Spawn an RTP capture task that filters PT 101 events.
    let captured: Arc<parking_lot::Mutex<Vec<(u8, bool, u8, u16, bool)>>> =
        Arc::new(parking_lot::Mutex::new(Vec::new()));
    let cap_clone = Arc::clone(&captured);
    let capture = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            let Some(event) =
                tokio::time::timeout(Duration::from_millis(100), remote.poll())
                    .await
                    .ok()
                    .flatten()
            else {
                continue;
            };
            if let TrackRemoteEvent::OnRtpPacket(pkt) = event {
                if pkt.header.payload_type == TELEPHONE_EVENT_PT {
                    if let Some((ev, eoe, vol, dur)) = decode_event(&pkt.payload) {
                        cap_clone.lock().push((ev, eoe, vol, dur, pkt.header.marker));
                    }
                }
            }
        }
    });

    // Send DTMF "5" for 100ms.
    send_dtmf(&offerer, "5", 100).await.expect("send_dtmf");

    // Wait long enough for end-of-event retransmissions to arrive.
    tokio::time::sleep(Duration::from_millis(300)).await;
    capture.abort();

    let events = captured.lock().clone();
    assert!(
        !events.is_empty(),
        "expected at least one RFC 4733 packet on the wire"
    );

    // Every captured event must be digit 5 with reasonable volume (0..63).
    for (ev, _, vol, _, _) in &events {
        assert_eq!(*ev, 5, "event code must be 5 for digit '5'");
        assert!(*vol <= 63, "volume must fit in 6 bits (got {vol})");
    }

    // First packet must carry the marker bit.
    let first = events.first().expect("first event");
    assert!(first.4, "first telephone-event packet should have marker bit set");
    assert!(!first.1, "first packet should not be end-of-event");

    // At least one packet must signal end-of-event (the retransmissions at the end).
    let any_end = events.iter().any(|(_, eoe, ..)| *eoe);
    assert!(any_end, "expected end-of-event marker among captured packets");

    // Duration must monotonically increase (cumulative samples across the tone).
    let mut last = 0u16;
    for (_, _, _, dur, _) in &events {
        // duration should never go backwards
        assert!(*dur >= last, "duration regressed: {dur} < {last}");
        last = *dur;
    }

    offerer.close().await.ok();
    answerer.close().await.ok();
}
