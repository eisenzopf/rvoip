//! H3 + D1: RFC 4733 DTMF wire-format capture — confirms `send_dtmf` actually
//! generates correct telephone-event RTP packets on the wire, end-to-end
//! through SRTP on a real loopback.
//!
//! D1 attached a dedicated `TrackLocalStaticRTP` for PT 101 on its own SSRC.
//! The offerer's SDP carries two audio m-lines (Opus stream + DTMF stream),
//! and PT 101 packets arrive on the answerer's dedicated DTMF remote track —
//! so the test discovers all audio remote tracks and captures from each.

use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::media::dtmf::send_dtmf;
use rvoip_webrtc::peer::connect_loopback;
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
async fn send_dtmf_emits_rfc4733_telephone_events() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (offerer, answerer) = connect_loopback(&WebRtcConfig::loopback())
        .await
        .expect("loopback");

    // D1 — the answerer has *two* audio transceivers after negotiation
    // (Opus stream + DTMF stream). PT 101 arrives only on the DTMF
    // transceiver's receiver, so we spawn a watcher that drains every
    // remote audio track from the handler's on_track channel and polls
    // each one. New tracks may appear after we start sending PT 101
    // (webrtc-rs fires on_track on first inbound RTP), so the watcher
    // keeps polling until the deadline.
    let captured: Arc<parking_lot::Mutex<Vec<(u8, bool, u8, u16, bool)>>> =
        Arc::new(parking_lot::Mutex::new(Vec::new()));
    let cap_clone = Arc::clone(&captured);
    let answerer_watch = Arc::clone(&answerer);
    let watcher = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut pollers: Vec<tokio::task::JoinHandle<()>> = Vec::new();
        while tokio::time::Instant::now() < deadline {
            if let Some(track) = answerer_watch.try_recv_remote_track().await {
                let cap = Arc::clone(&cap_clone);
                pollers.push(tokio::spawn(async move {
                    loop {
                        let Some(event) = tokio::time::timeout(
                            Duration::from_millis(100),
                            track.poll(),
                        )
                        .await
                        .ok()
                        .flatten() else {
                            continue;
                        };
                        if let TrackRemoteEvent::OnRtpPacket(pkt) = event {
                            if pkt.header.payload_type == TELEPHONE_EVENT_PT {
                                if let Some((ev, eoe, vol, dur)) =
                                    decode_event(&pkt.payload)
                                {
                                    cap.lock().push((
                                        ev,
                                        eoe,
                                        vol,
                                        dur,
                                        pkt.header.marker,
                                    ));
                                }
                            }
                        }
                    }
                }));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        for h in pollers {
            h.abort();
        }
    });

    // Send DTMF "5" for 100ms — first PT 101 packet triggers on_track
    // on the answerer's DTMF transceiver.
    send_dtmf(&offerer, "5", 100).await.expect("send_dtmf");

    // Wait long enough for end-of-event retransmissions to arrive.
    tokio::time::sleep(Duration::from_millis(500)).await;
    watcher.abort();

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
        assert!(*dur >= last, "duration regressed: {dur} < {last}");
        last = *dur;
    }

    offerer.close().await.ok();
    answerer.close().await.ok();
}
