//! H3 + D1: RFC 4733 DTMF wire-format capture — confirms `send_dtmf` actually
//! generates correct telephone-event RTP packets on the wire, end-to-end
//! through SRTP on a real loopback.
//!
//! DTMF is an alternate payload on the same negotiated audio source as Opus.
//! The offer therefore carries one audio m-line, SSRC, and sequence/timestamp
//! owner. The tests discover every audio remote track so they remain
//! insensitive to receiver-side demux shape.

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::capability::CodecInfo;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::MediaStream;
use rvoip_webrtc::media::dtmf::{send_dtmf, OutboundDtmfNegotiation, TelephoneEventCodec};
use rvoip_webrtc::media::from_tracks_with_dtmf_codecs;
use rvoip_webrtc::peer::{connect_loopback, PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;
use webrtc::media_stream::track_remote::TrackRemoteEvent;

const TELEPHONE_EVENT_PT: u8 = 101;

fn retain_only_48khz_telephone_event(sdp: &str) -> String {
    let mut output = String::with_capacity(sdp.len());
    for line in sdp.lines() {
        if let Some(rest) = line.strip_prefix("m=audio ") {
            let mut fields = rest.split_whitespace().collect::<Vec<_>>();
            if fields.len() >= 2 {
                // `rest` starts after the media kind, so only port and proto
                // are structural. Filter every following format, including
                // the first payload type.
                let mut retained = fields.drain(..2).collect::<Vec<_>>();
                retained.extend(
                    fields
                        .into_iter()
                        .filter(|payload_type| !matches!(*payload_type, "101" | "126")),
                );
                output.push_str("m=audio ");
                output.push_str(&retained.join(" "));
                output.push_str("\r\n");
                continue;
            }
        }
        if line.starts_with("a=rtpmap:101 ")
            || line.starts_with("a=fmtp:101 ")
            || line.starts_with("a=rtpmap:126 ")
            || line.starts_with("a=fmtp:126 ")
        {
            continue;
        }
        output.push_str(line);
        output.push_str("\r\n");
    }
    output
}

/// Drop every SDES MID `a=extmap` line, reproducing an endpoint such as
/// Amazon Connect that never negotiates the MID header extension. The `a=mid:`
/// attribute itself is retained so BUNDLE grouping is unchanged.
fn strip_sdes_mid_extmap(sdp: &str) -> String {
    let mut output = String::with_capacity(sdp.len());
    for line in sdp.lines() {
        if line.starts_with("a=extmap:") && line.contains("urn:ietf:params:rtp-hdrext:sdes:mid") {
            continue;
        }
        output.push_str(line);
        output.push_str("\r\n");
    }
    output
}

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

    // Opus and telephone-event share one negotiated audio source. Receiver
    // implementations may expose a remote track only after the first RTP
    // packet arrives, so keep discovering and polling until the deadline.
    let expected_ssrc = offerer.local_audio_ssrc().expect("primary audio SSRC");
    let captured: Arc<parking_lot::Mutex<Vec<(u8, bool, u8, u16, bool, u32)>>> =
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
                        let Some(event) =
                            tokio::time::timeout(Duration::from_millis(100), track.poll())
                                .await
                                .ok()
                                .flatten()
                        else {
                            continue;
                        };
                        if let TrackRemoteEvent::OnRtpPacket(pkt) = event {
                            if pkt.header.payload_type == TELEPHONE_EVENT_PT {
                                if let Some((ev, eoe, vol, dur)) = decode_event(&pkt.payload) {
                                    cap.lock().push((
                                        ev,
                                        eoe,
                                        vol,
                                        dur,
                                        pkt.header.marker,
                                        pkt.header.ssrc,
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

    // Send DTMF "5" for 100ms on the already-negotiated primary audio source.
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
    for (ev, _, vol, _, _, ssrc) in &events {
        assert_eq!(*ev, 5, "event code must be 5 for digit '5'");
        assert!(*vol <= 63, "volume must fit in 6 bits (got {vol})");
        assert_eq!(
            *ssrc, expected_ssrc,
            "RFC 4733 must use the primary audio source"
        );
    }

    // First packet must carry the marker bit.
    let first = events.first().expect("first event");
    assert!(
        first.4,
        "first telephone-event packet should have marker bit set"
    );
    assert!(!first.1, "first packet should not be end-of-event");

    // At least one packet must signal end-of-event (the retransmissions at the end).
    let any_end = events.iter().any(|(_, eoe, ..)| *eoe);
    assert!(
        any_end,
        "expected end-of-event marker among captured packets"
    );

    // Duration must monotonically increase (cumulative samples across the tone).
    let mut last = 0u16;
    for (_, _, _, dur, _, _) in &events {
        assert!(*dur >= last, "duration regressed: {dur} < {last}");
        last = *dur;
    }

    offerer.close().await.ok();
    answerer.close().await.ok();
}

#[tokio::test]
async fn negotiated_pt110_48khz_reaches_the_remote_peer() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let receiver = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("receiver");
    receiver
        .prepare_receive_only_offer()
        .await
        .expect("receive-only offer");
    let original_offer = receiver.create_offer_and_gather().await.expect("offer");
    let offer = retain_only_48khz_telephone_event(&original_offer);
    assert!(offer.contains("a=rtpmap:110 telephone-event/48000"));
    assert!(!offer.contains("a=rtpmap:101 telephone-event/8000"));
    assert!(!offer.contains("a=rtpmap:126 telephone-event/8000"));

    let sender = RvoipPeerConnection::new(&config, PeerRole::Answerer)
        .await
        .expect("sender");
    let answer = sender
        .accept_offer_and_gather(&offer)
        .await
        .expect("48 kHz answer");
    assert!(answer.contains("a=rtpmap:110 telephone-event/48000"));
    receiver
        .set_remote_answer(&answer)
        .await
        .expect("install answer");
    assert_eq!(
        sender.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Negotiated(TelephoneEventCodec::new(110, 48_000))
    );
    let expected_mid = sender
        .negotiated_outbound_audio_mid()
        .expect("negotiated outbound audio MID");
    let expected_mid_id = sender
        .negotiated_outbound_audio_mid_extension_id()
        .expect("negotiated outbound audio MID extension ID");
    let expected_ssrc = sender.local_audio_ssrc().expect("primary audio SSRC");

    let timeout = Duration::from_secs(config.connection_timeout_secs);
    tokio::try_join!(
        receiver.wait_connected(timeout),
        sender.wait_connected(timeout)
    )
    .expect("connected peers");

    let captured: Arc<parking_lot::Mutex<Vec<(u8, bool, u16, bool, Option<Vec<u8>>, u32)>>> =
        Arc::new(parking_lot::Mutex::new(Vec::new()));
    let captured_for_watcher = Arc::clone(&captured);
    let receiver_for_watcher = Arc::clone(&receiver);
    let watcher = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut pollers = Vec::new();
        while tokio::time::Instant::now() < deadline {
            if let Some(track) = receiver_for_watcher.try_recv_remote_track().await {
                let captured = Arc::clone(&captured_for_watcher);
                pollers.push(tokio::spawn(async move {
                    loop {
                        let Some(TrackRemoteEvent::OnRtpPacket(packet)) =
                            tokio::time::timeout(Duration::from_millis(100), track.poll())
                                .await
                                .ok()
                                .flatten()
                        else {
                            continue;
                        };
                        if packet.header.payload_type == 110 {
                            if let Some((event, end, _, duration)) = decode_event(&packet.payload) {
                                captured.lock().push((
                                    event,
                                    end,
                                    duration,
                                    packet.header.marker,
                                    packet
                                        .header
                                        .get_extension(expected_mid_id)
                                        .map(|payload| payload.to_vec()),
                                    packet.header.ssrc,
                                ));
                            }
                        }
                    }
                }));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        for poller in pollers {
            poller.abort();
        }
    });

    send_dtmf(&sender, "6", 120)
        .await
        .expect("negotiated PT110 DTMF");
    tokio::time::sleep(Duration::from_millis(500)).await;
    watcher.abort();

    let events = captured.lock().clone();
    assert!(!events.is_empty(), "expected PT110 packets on the wire");
    assert!(events.iter().all(|(event, ..)| *event == 6));
    assert!(events.first().is_some_and(|event| event.3));
    assert!(events
        .iter()
        .any(|(_, end, duration, _, _, _)| { *end && *duration == 5_760 }));
    assert!(
        events
            .iter()
            .all(|event| event.4.as_deref() == Some(expected_mid.as_bytes())),
        "every telephone-event packet must carry the exact negotiated MID bytes"
    );
    assert!(
        events.iter().all(|event| event.5 == expected_ssrc),
        "PT110 must use the primary audio source"
    );

    receiver.close().await.ok();
    sender.close().await.ok();
}

#[tokio::test]
async fn public_media_stream_decodes_the_negotiated_dynamic_dtmf_mapping() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let receiver = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("receiver");
    receiver
        .prepare_receive_only_offer()
        .await
        .expect("receive-only offer");
    let original_offer = receiver.create_offer_and_gather().await.expect("offer");
    let offer = retain_only_48khz_telephone_event(&original_offer);
    let dtmf_codec = TelephoneEventCodec::new(110, 48_000);

    let sender = RvoipPeerConnection::new(&config, PeerRole::Answerer)
        .await
        .expect("sender");
    let answer = sender
        .accept_offer_and_gather(&offer)
        .await
        .expect("48 kHz answer");
    assert_eq!(
        sender.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Negotiated(dtmf_codec)
    );
    receiver
        .set_remote_answer(&answer)
        .await
        .expect("install answer");

    let timeout = Duration::from_secs(config.connection_timeout_secs);
    tokio::try_join!(
        receiver.wait_connected(timeout),
        sender.wait_connected(timeout)
    )
    .expect("connected peers");

    // The remote track is opened by the first DTMF RTP packet. Start its
    // waiter before sending, then give the public media constructor ownership
    // of that one track so it can decode the queued PT 110 packets itself.
    let receiver_for_track = Arc::clone(&receiver);
    let remote_track = tokio::spawn(async move {
        receiver_for_track
            .wait_remote_track(Duration::from_secs(5))
            .await
    });
    send_dtmf(&sender, "6", 120)
        .await
        .expect("negotiated PT110 DTMF");
    let remote_track = remote_track
        .await
        .expect("remote-track waiter")
        .expect("remote DTMF track");

    let (dtmf_tx, mut dtmf_rx) = tokio::sync::mpsc::channel(4);
    let stream = from_tracks_with_dtmf_codecs(
        StreamId::new(),
        CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: None,
            payload_type: None,
        },
        receiver.local_audio_track().expect("receiver audio track"),
        receiver.local_audio_ssrc().expect("receiver audio SSRC"),
        111,
        Some(remote_track),
        Some(dtmf_tx),
        [dtmf_codec],
    );
    let event = tokio::time::timeout(Duration::from_secs(2), dtmf_rx.recv())
        .await
        .expect("dynamic DTMF decode timeout")
        .expect("DTMF receiver remains live");
    assert_eq!(event.digit, '6');
    assert_eq!(event.duration_ms, 120);

    stream.close().await.expect("close media stream");
    receiver.close().await.ok();
    sender.close().await.ok();
}

/// Regression: an endpoint that does not negotiate the SDES MID header
/// extension must still receive RFC 4733 telephone events.
///
/// Since telephone events were moved onto the primary negotiated audio SSRC,
/// they are exactly as demuxable as primary audio. Primary audio is already
/// allowed to be written without a MID, so DTMF must not fail closed where
/// audio succeeds -- otherwise audio reaches the peer and IVR digits do not.
#[tokio::test]
async fn dtmf_reaches_a_peer_that_never_negotiates_the_sdes_mid_extension() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();

    let receiver = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("receiver");
    receiver
        .prepare_receive_only_offer()
        .await
        .expect("receive-only offer");
    let original_offer = receiver.create_offer_and_gather().await.expect("offer");
    assert!(
        original_offer.contains("urn:ietf:params:rtp-hdrext:sdes:mid"),
        "baseline offer must advertise the MID extension so stripping is meaningful"
    );
    let offer = strip_sdes_mid_extmap(&original_offer);
    assert!(!offer.contains("urn:ietf:params:rtp-hdrext:sdes:mid"));
    assert!(
        offer.contains("a=mid:"),
        "BUNDLE grouping must be preserved"
    );

    let sender = RvoipPeerConnection::new(&config, PeerRole::Answerer)
        .await
        .expect("sender");
    let answer = sender
        .accept_offer_and_gather(&offer)
        .await
        .expect("answer without the MID extension");
    receiver
        .set_remote_answer(&answer)
        .await
        .expect("install answer");

    // Preconditions: DTMF is negotiated, but no MID was agreed.
    assert!(
        matches!(
            sender.outbound_dtmf_negotiation(),
            OutboundDtmfNegotiation::Negotiated(_)
        ),
        "telephone-event must still negotiate without the MID extension"
    );
    assert!(
        sender.negotiated_outbound_audio_mid().is_none(),
        "this test is only meaningful when no MID is negotiated"
    );
    let expected_ssrc = sender.local_audio_ssrc().expect("primary audio SSRC");

    let timeout = Duration::from_secs(config.connection_timeout_secs);
    tokio::try_join!(
        receiver.wait_connected(timeout),
        sender.wait_connected(timeout)
    )
    .expect("connected peers");

    let captured: Arc<parking_lot::Mutex<Vec<(u8, bool, u32, bool)>>> =
        Arc::new(parking_lot::Mutex::new(Vec::new()));
    let captured_for_watcher = Arc::clone(&captured);
    let receiver_for_watcher = Arc::clone(&receiver);
    let watcher = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut pollers = Vec::new();
        while tokio::time::Instant::now() < deadline {
            if let Some(track) = receiver_for_watcher.try_recv_remote_track().await {
                let captured = Arc::clone(&captured_for_watcher);
                pollers.push(tokio::spawn(async move {
                    loop {
                        let Some(TrackRemoteEvent::OnRtpPacket(packet)) =
                            tokio::time::timeout(Duration::from_millis(100), track.poll())
                                .await
                                .ok()
                                .flatten()
                        else {
                            continue;
                        };
                        if packet.header.payload_type == TELEPHONE_EVENT_PT {
                            if let Some((event, end, _, _)) = decode_event(&packet.payload) {
                                captured.lock().push((
                                    event,
                                    end,
                                    packet.header.ssrc,
                                    packet.header.extension,
                                ));
                            }
                        }
                    }
                }));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        for poller in pollers {
            poller.abort();
        }
    });

    // The regression itself: this returned Err(IncompatibleCapabilities)
    // before the fix, without writing a single packet.
    send_dtmf(&sender, "7", 120)
        .await
        .expect("DTMF must not fail closed merely because no MID was negotiated");
    tokio::time::sleep(Duration::from_millis(500)).await;
    watcher.abort();

    let events = captured.lock().clone();
    assert!(
        !events.is_empty(),
        "expected telephone-event packets on the wire without a negotiated MID"
    );
    assert!(events.iter().all(|(event, ..)| *event == 7));
    assert!(
        events.iter().any(|(_, end, _, _)| *end),
        "expected an end-of-event packet"
    );
    assert!(
        events.iter().all(|event| event.2 == expected_ssrc),
        "telephone events must use the primary audio source"
    );
    assert!(
        events.iter().all(|event| !event.3),
        "no RTP header extension may be written when no MID was negotiated"
    );

    receiver.close().await.ok();
    sender.close().await.ok();
}
