//! Cross-layer integration tests: WebSocket transport → SIP dialog → media negotiation.
//!
//! These tests exercise the full path of SIP signaling with SDP bodies over
//! WebSocket transport, verifying that:
//!   - SDP offer bodies survive WS framing intact
//!   - SDP answer bodies can be sent back over WS
//!   - Multiple independent dialogs (Call-IDs) work over WS
//!   - Large multi-media SDP bodies are handled correctly
//!   - REGISTER → 200 OK flows work over WS
//!   - OPTIONS keep-alive probes work over WS (RFC 7118)

#![cfg(feature = "ws")]

use std::str::FromStr;
use std::time::Duration;

use tokio::time::timeout;

use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use rvoip_sip_core::types::method::Method;
use rvoip_sip_core::types::sdp::SdpSession;
use rvoip_sip_core::types::StatusCode;
use rvoip_sip_core::Message;
use rvoip_sip_transport::transport::{Transport, TransportEvent, WebSocketTransport};

// =============================================================================
// Helpers
// =============================================================================

const TIMEOUT_DUR: Duration = Duration::from_secs(5);

/// Bind a plain WS transport on a random port and return (transport, event_rx).
async fn bind_ws() -> (WebSocketTransport, tokio::sync::mpsc::Receiver<TransportEvent>) {
    WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
        .await
        .expect("should bind WS transport")
}

/// Wait for the next MessageReceived event within a timeout, returning the message.
async fn recv_message(
    rx: &mut tokio::sync::mpsc::Receiver<TransportEvent>,
    wait: Duration,
) -> Message {
    timeout(wait, async {
        loop {
            match rx.recv().await {
                Some(TransportEvent::MessageReceived { message, .. }) => return message,
                Some(_) => continue,
                None => panic!("event channel closed unexpectedly"),
            }
        }
    })
    .await
    .expect("timed out waiting for SIP message")
}

/// Build a basic SDP offer for audio with PCMU and PCMA codecs.
fn build_audio_sdp_offer() -> SdpSession {
    SdpBuilder::new("WebSocket Call")
        .origin("-", "1000000001", "1", "IN", "IP4", "192.168.1.100")
        .connection("IN", "IP4", "192.168.1.100")
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()
        .expect("valid SDP offer")
}

/// Build an SDP answer with a single audio codec (PCMU).
fn build_audio_sdp_answer() -> SdpSession {
    SdpBuilder::new("WebSocket Call Answer")
        .origin("-", "2000000002", "1", "IN", "IP4", "192.168.1.200")
        .connection("IN", "IP4", "192.168.1.200")
        .time("0", "0")
        .media_audio(30000, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()
        .expect("valid SDP answer")
}

/// Build a multi-media SDP (audio + video) for large-body tests.
fn build_multi_media_sdp() -> SdpSession {
    SdpBuilder::new("Multi-Media Session")
        .origin("-", "3000000003", "1", "IN", "IP4", "10.0.0.1")
        .connection("IN", "IP4", "10.0.0.1")
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8", "101"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .rtpmap("101", "telephone-event/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .media_video(51372, "RTP/AVP")
            .formats(&["96", "97"])
            .rtpmap("96", "VP8/90000")
            .rtpmap("97", "H264/90000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()
        .expect("valid multi-media SDP")
}

/// Build a SIP INVITE message carrying an SDP body.
fn build_invite_with_sdp(call_id: &str, sdp: &SdpSession) -> Message {
    let sdp_text = sdp.to_string();
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some("ws-sdp-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .via("127.0.0.1:5060", "WS", Some("z9hG4bK-ws-sdp-inv"))
        .max_forwards(70)
        .content_type("application/sdp")
        .body(sdp_text)
        .build()
        .into()
}

/// Build a 200 OK response carrying an SDP answer body.
fn build_200_ok_with_sdp(call_id: &str, sdp: &SdpSession) -> Message {
    let sdp_text = sdp.to_string();
    SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
        .from("Alice", "sip:alice@example.com", Some("ws-sdp-tag"))
        .to("Bob", "sip:bob@example.com", Some("ws-to-tag"))
        .call_id(call_id)
        .cseq(1, Method::Invite)
        .via("127.0.0.1:5060", "WS", Some("z9hG4bK-ws-sdp-inv"))
        .content_type("application/sdp")
        .body(sdp_text)
        .build()
        .into()
}

/// Build a REGISTER request (no body).
fn build_register(call_id: &str) -> Message {
    SimpleRequestBuilder::new(Method::Register, "sip:example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some("ws-reg-tag"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .via("127.0.0.1:5060", "WS", Some("z9hG4bK-ws-reg"))
        .max_forwards(70)
        .build()
        .into()
}

/// Build a 200 OK for a REGISTER (no body).
fn build_register_200(call_id: &str) -> Message {
    SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
        .from("Alice", "sip:alice@example.com", Some("ws-reg-tag"))
        .to("Alice", "sip:alice@example.com", Some("ws-reg-to-tag"))
        .call_id(call_id)
        .cseq(1, Method::Register)
        .via("127.0.0.1:5060", "WS", Some("z9hG4bK-ws-reg"))
        .build()
        .into()
}

/// Build an OPTIONS request (keep-alive probe per RFC 7118).
fn build_options(call_id: &str) -> Message {
    SimpleRequestBuilder::new(Method::Options, "sip:proxy.example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some("ws-opt-tag"))
        .to("Proxy", "sip:proxy@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .via("127.0.0.1:5060", "WS", Some("z9hG4bK-ws-options"))
        .max_forwards(70)
        .build()
        .into()
}

// =============================================================================
// Test 1: WS SIP INVITE with SDP body — offer survives framing intact
// =============================================================================

#[tokio::test]
async fn test_ws_invite_with_sdp_body_roundtrip() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, _client_rx) = bind_ws().await;

    let sdp_offer = build_audio_sdp_offer();
    let sdp_offer_text = sdp_offer.to_string();
    let call_id = "ws-sdp-invite-001@example.com";
    let msg = build_invite_with_sdp(call_id, &sdp_offer);

    client.send_message(msg, server_addr).await.expect("send INVITE with SDP");

    let received = recv_message(&mut server_rx, TIMEOUT_DUR).await;

    if let Message::Request(req) = received {
        // Verify SIP layer
        assert_eq!(req.method(), Method::Invite, "should be INVITE");
        assert_eq!(
            req.call_id().expect("Call-ID").to_string(),
            call_id,
            "Call-ID mismatch"
        );

        // Verify SDP body survived WS framing
        let body = req.body();
        assert!(!body.is_empty(), "INVITE body should not be empty");

        let body_str = std::str::from_utf8(body).expect("body should be valid UTF-8");
        assert!(
            body_str.contains("m=audio"),
            "SDP body should contain audio media line"
        );

        // Parse the SDP from the received body
        let parsed_sdp = SdpSession::from_str(body_str)
            .expect("received SDP should parse correctly");

        // Verify media descriptions survived intact
        assert_eq!(
            parsed_sdp.media_descriptions.len(),
            1,
            "should have one audio media description"
        );

        let audio_md = &parsed_sdp.media_descriptions[0];
        assert_eq!(audio_md.media, "audio", "media type should be audio");
        assert_eq!(audio_md.port, 49170, "audio port should match offer");
        assert_eq!(audio_md.protocol, "RTP/AVP", "protocol should match");

        // Verify codecs are present
        assert!(
            audio_md.formats.contains(&"0".to_string()),
            "should contain PCMU (PT 0)"
        );
        assert!(
            audio_md.formats.contains(&"8".to_string()),
            "should contain PCMA (PT 8)"
        );

        // Verify the SDP text is byte-for-byte identical
        assert_eq!(
            body_str.trim(),
            sdp_offer_text.trim(),
            "SDP body should match the original offer text"
        );
    } else {
        panic!("expected SIP request, got response");
    }

    client.close().await.expect("client close");
    server.close().await.expect("server close");
}

// =============================================================================
// Test 2: WS SIP REGISTER → 200 OK flow
// =============================================================================

#[tokio::test]
async fn test_ws_register_200ok_flow() {
    // Server (registrar side) and client (UA side) each get a WS transport.
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, mut client_rx) = bind_ws().await;
    let client_addr = client.local_addr().expect("client addr");

    let call_id = "ws-register-flow-001@example.com";

    // Step 1: Client sends REGISTER to server
    let register = build_register(call_id);
    client
        .send_message(register, server_addr)
        .await
        .expect("send REGISTER");

    // Step 2: Server receives the REGISTER
    let received = recv_message(&mut server_rx, TIMEOUT_DUR).await;
    if let Message::Request(req) = &received {
        assert_eq!(req.method(), Method::Register, "should be REGISTER");
        assert_eq!(
            req.call_id().expect("Call-ID").to_string(),
            call_id,
        );
    } else {
        panic!("expected REGISTER request");
    }

    // Step 3: Server sends 200 OK back to client
    let ok_response = build_register_200(call_id);
    server
        .send_message(ok_response, client_addr)
        .await
        .expect("send 200 OK response");

    // Step 4: Client receives the 200 OK
    let response = recv_message(&mut client_rx, TIMEOUT_DUR).await;
    if let Message::Response(resp) = response {
        assert_eq!(
            resp.status_code(),
            200u16,
            "should be 200 OK"
        );
    } else {
        panic!("expected SIP response, got request");
    }

    client.close().await.expect("client close");
    server.close().await.expect("server close");
}

// =============================================================================
// Test 3: WS SIP OPTIONS keep-alive (RFC 7118)
// =============================================================================

#[tokio::test]
async fn test_ws_options_keepalive() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, _client_rx) = bind_ws().await;

    let call_id = "ws-options-keepalive-001@example.com";
    let options = build_options(call_id);

    client
        .send_message(options, server_addr)
        .await
        .expect("send OPTIONS");

    let received = recv_message(&mut server_rx, TIMEOUT_DUR).await;
    if let Message::Request(req) = received {
        assert_eq!(req.method(), Method::Options, "should be OPTIONS");
        assert_eq!(
            req.call_id().expect("Call-ID").to_string(),
            call_id,
        );
        // OPTIONS as keep-alive should be lightweight — verify minimal headers are intact
        assert!(req.from().is_some(), "OPTIONS should have From header");
        assert!(req.to().is_some(), "OPTIONS should have To header");
        assert!(!req.via_headers().is_empty(), "OPTIONS should have Via header");
    } else {
        panic!("expected OPTIONS request");
    }

    client.close().await.expect("client close");
    server.close().await.expect("server close");
}

// =============================================================================
// Test 4: SDP offer/answer over WS — full negotiation simulation
// =============================================================================

#[tokio::test]
async fn test_ws_sdp_offer_answer_negotiation() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, mut client_rx) = bind_ws().await;
    let client_addr = client.local_addr().expect("client addr");

    let call_id = "ws-sdp-negotiate-001@example.com";

    // Step 1: Client sends INVITE with SDP offer
    let sdp_offer = build_audio_sdp_offer();
    let invite = build_invite_with_sdp(call_id, &sdp_offer);
    client
        .send_message(invite, server_addr)
        .await
        .expect("send INVITE with SDP offer");

    // Step 2: Server receives INVITE and parses the SDP offer
    let received = recv_message(&mut server_rx, TIMEOUT_DUR).await;
    let offer_sdp = if let Message::Request(req) = &received {
        assert_eq!(req.method(), Method::Invite, "should be INVITE");
        let body_str = std::str::from_utf8(req.body()).expect("UTF-8 body");
        let parsed = SdpSession::from_str(body_str).expect("parse offer SDP");
        assert_eq!(parsed.media_descriptions.len(), 1, "offer has 1 media line");
        assert_eq!(parsed.media_descriptions[0].media, "audio");
        // Offer contains PCMU (0) and PCMA (8)
        assert!(parsed.media_descriptions[0].formats.contains(&"0".to_string()));
        assert!(parsed.media_descriptions[0].formats.contains(&"8".to_string()));
        parsed
    } else {
        panic!("expected INVITE request");
    };

    // Step 3: Server sends 200 OK with SDP answer (selecting PCMU only)
    let sdp_answer = build_audio_sdp_answer();
    let ok_response = build_200_ok_with_sdp(call_id, &sdp_answer);
    server
        .send_message(ok_response, client_addr)
        .await
        .expect("send 200 OK with SDP answer");

    // Step 4: Client receives 200 OK and parses the SDP answer
    let response = recv_message(&mut client_rx, TIMEOUT_DUR).await;
    if let Message::Response(resp) = response {
        assert_eq!(resp.status_code(), 200u16, "should be 200 OK");

        let body_str = std::str::from_utf8(resp.body()).expect("UTF-8 body");
        assert!(
            !body_str.is_empty(),
            "200 OK body should contain SDP answer"
        );

        let answer_sdp = SdpSession::from_str(body_str).expect("parse answer SDP");
        assert_eq!(
            answer_sdp.media_descriptions.len(),
            1,
            "answer has 1 media line"
        );
        assert_eq!(answer_sdp.media_descriptions[0].media, "audio");
        // Answer selected only PCMU (0)
        assert!(
            answer_sdp.media_descriptions[0].formats.contains(&"0".to_string()),
            "answer should include PCMU"
        );
        assert_eq!(
            answer_sdp.media_descriptions[0].port, 30000,
            "answer port should be 30000"
        );

        // Verify the answer is a valid subset of the offer codecs
        for fmt in &answer_sdp.media_descriptions[0].formats {
            assert!(
                offer_sdp.media_descriptions[0].formats.contains(fmt),
                "answer format {} should be present in offer",
                fmt
            );
        }
    } else {
        panic!("expected SIP response");
    }

    client.close().await.expect("client close");
    server.close().await.expect("server close");
}

// =============================================================================
// Test 5: Multiple dialogs over the same WS connection
// =============================================================================

#[tokio::test]
async fn test_ws_multiple_dialogs_over_same_connection() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, _client_rx) = bind_ws().await;

    let sdp = build_audio_sdp_offer();

    let call_id_1 = "ws-dialog-alpha-001@example.com";
    let call_id_2 = "ws-dialog-beta-002@example.com";

    // Send two INVITEs with different Call-IDs over the same WS connection
    let invite_1 = build_invite_with_sdp(call_id_1, &sdp);
    let invite_2 = build_invite_with_sdp(call_id_2, &sdp);

    client
        .send_message(invite_1, server_addr)
        .await
        .expect("send INVITE 1");
    client
        .send_message(invite_2, server_addr)
        .await
        .expect("send INVITE 2");

    // Receive both messages
    let msg1 = recv_message(&mut server_rx, TIMEOUT_DUR).await;
    let msg2 = recv_message(&mut server_rx, TIMEOUT_DUR).await;

    // Extract Call-IDs and verify both are present
    let mut received_call_ids: Vec<String> = Vec::new();
    for msg in [msg1, msg2] {
        if let Message::Request(req) = msg {
            assert_eq!(req.method(), Method::Invite);
            received_call_ids.push(req.call_id().expect("Call-ID").to_string());

            // Each INVITE should have a valid SDP body
            let body = req.body();
            assert!(!body.is_empty(), "INVITE body should not be empty");
            let body_str = std::str::from_utf8(body).expect("UTF-8 body");
            let parsed = SdpSession::from_str(body_str).expect("parse SDP");
            assert_eq!(parsed.media_descriptions.len(), 1);
        } else {
            panic!("expected request");
        }
    }

    received_call_ids.sort();
    let mut expected = vec![call_id_1.to_string(), call_id_2.to_string()];
    expected.sort();
    assert_eq!(
        received_call_ids, expected,
        "both dialog Call-IDs should arrive correctly"
    );

    client.close().await.expect("client close");
    server.close().await.expect("server close");
}

// =============================================================================
// Test 6: Large multi-media SDP body over WS
// =============================================================================

#[tokio::test]
async fn test_ws_large_multi_media_sdp() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, _client_rx) = bind_ws().await;

    let multi_sdp = build_multi_media_sdp();
    let sdp_text = multi_sdp.to_string();
    let call_id = "ws-large-sdp-001@example.com";

    // Verify the SDP is non-trivially large (audio + video lines)
    assert!(
        sdp_text.len() > 200,
        "multi-media SDP should be reasonably large, got {} bytes",
        sdp_text.len()
    );

    let msg = build_invite_with_sdp(call_id, &multi_sdp);
    client
        .send_message(msg, server_addr)
        .await
        .expect("send INVITE with large SDP");

    let received = recv_message(&mut server_rx, TIMEOUT_DUR).await;
    if let Message::Request(req) = received {
        assert_eq!(req.method(), Method::Invite);

        let body = req.body();
        assert!(!body.is_empty(), "body should not be empty");

        let body_str = std::str::from_utf8(body).expect("UTF-8 body");

        // Verify the full multi-media SDP survived WS framing
        let parsed = SdpSession::from_str(body_str).expect("parse multi-media SDP");

        assert_eq!(
            parsed.media_descriptions.len(),
            2,
            "should have 2 media descriptions (audio + video)"
        );

        // Verify audio section
        let audio_md = parsed
            .media_descriptions
            .iter()
            .find(|md| md.media == "audio")
            .expect("should have audio media");
        assert_eq!(audio_md.port, 49170);
        assert!(audio_md.formats.contains(&"0".to_string()), "PCMU");
        assert!(audio_md.formats.contains(&"8".to_string()), "PCMA");
        assert!(
            audio_md.formats.contains(&"101".to_string()),
            "telephone-event"
        );

        // Verify video section
        let video_md = parsed
            .media_descriptions
            .iter()
            .find(|md| md.media == "video")
            .expect("should have video media");
        assert_eq!(video_md.port, 51372);
        assert!(video_md.formats.contains(&"96".to_string()), "VP8");
        assert!(video_md.formats.contains(&"97".to_string()), "H264");

        // Verify SDP text integrity
        assert_eq!(
            body_str.trim(),
            sdp_text.trim(),
            "multi-media SDP should survive WS framing intact"
        );
    } else {
        panic!("expected SIP request");
    }

    client.close().await.expect("client close");
    server.close().await.expect("server close");
}
