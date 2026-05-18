//! End-to-end raw-bytes preservation tests for the transport layer.
//!
//! Verifies the SIP_API_DESIGN_2 §7.5 contract: every shipping
//! transport (UDP / TCP / TLS / WebSocket) surfaces the original
//! wire bytes on `TransportEvent::MessageReceived.raw_bytes`, and
//! `Transport::send_message_raw` puts those bytes back on the wire
//! verbatim (the SBC pass-through / STIR-SHAKEN preservation use
//! case from RFC 8224).

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_transport::transport::udp::UdpTransport;
use rvoip_sip_transport::{Transport, TransportEvent};
use tokio::sync::mpsc;

/// Build a SIP REGISTER message with deliberately non-canonical
/// whitespace and mixed-case header names. If sip-core's `Display`
/// impl re-canonicalises (which it does), the bytes received on the
/// far side will differ from the bytes sent. This is exactly the
/// fidelity STIR/SHAKEN Identity verification needs to survive.
fn non_canonical_register_bytes() -> bytes::Bytes {
    let msg = concat!(
        "REGISTER sip:registrar.example.com SIP/2.0\r\n",
        "Via:  SIP/2.0/UDP  127.0.0.1:5060 ;branch=z9hG4bK-test\r\n",
        "from: <sip:alice@example.com>;tag=42\r\n",
        "TO: <sip:alice@example.com>\r\n",
        "Call-ID:  preserve-bytes-1@example.com\r\n",
        "cseq: 1   REGISTER\r\n",
        "Contact: <sip:alice@127.0.0.1:5060>\r\n",
        "Max-Forwards: 70\r\n",
        "Identity: \"abc.def.ghi\";info=<https://example.com/cert.pem>\r\n",
        "Content-Length: 0\r\n",
        "\r\n",
    );
    bytes::Bytes::from_static(msg.as_bytes())
}

async fn bind_udp_pair() -> (
    std::sync::Arc<UdpTransport>,
    mpsc::Receiver<TransportEvent>,
    std::sync::Arc<UdpTransport>,
    SocketAddr,
) {
    let (server, server_rx) = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), None)
        .await
        .expect("bind server udp");
    let server_addr = server.local_addr().expect("server local_addr");

    let (client, _client_rx) = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), None)
        .await
        .expect("bind client udp");

    (
        std::sync::Arc::new(server),
        server_rx,
        std::sync::Arc::new(client),
        server_addr,
    )
}

/// UDP transport surfaces the exact bytes the datagram carried on
/// `MessageReceived.raw_bytes`, byte-for-byte. Non-canonical
/// whitespace / case survives — that's the STIR/SHAKEN guarantee.
#[tokio::test]
async fn udp_raw_bytes_match_wire_exactly() {
    let (_server, mut server_rx, client, server_addr) = bind_udp_pair().await;
    let original = non_canonical_register_bytes();

    client
        .send_message_raw(original.clone(), server_addr)
        .await
        .expect("send_message_raw");

    let event = tokio::time::timeout(Duration::from_secs(2), server_rx.recv())
        .await
        .expect("server receive timeout")
        .expect("server channel closed");

    match event {
        TransportEvent::MessageReceived {
            raw_bytes: Some(bytes),
            message,
            ..
        } => {
            assert_eq!(
                bytes.as_ref(),
                original.as_ref(),
                "raw_bytes must match the original wire form byte-for-byte"
            );

            // Sanity check that re-serializing the parsed Message
            // would have normalised away the non-canonical bits —
            // this proves raw_bytes is the only way to recover the
            // upstream form (STIR/SHAKEN's hard requirement).
            let reserialized = message.to_bytes();
            assert_ne!(
                bytes.as_ref(),
                reserialized.as_slice(),
                "sip-core normalisation would have re-serialised the message; \
                 raw_bytes proves we kept the upstream form"
            );
        }
        other => panic!("expected MessageReceived with raw_bytes, got {:?}", other),
    }
}

/// `Transport::send_message_raw` ships a pre-built buffer verbatim
/// and the receiver sees identical bytes. This is the SBC pass-
/// through / replay-tooling primitive.
#[tokio::test]
async fn udp_send_message_raw_round_trip() {
    let (_server, mut server_rx, client, server_addr) = bind_udp_pair().await;

    // Hand-craft a request a typed builder wouldn't produce —
    // deliberate space variation in CSeq, ordering of header
    // parameters. send_message_raw must not normalise.
    let custom = bytes::Bytes::from_static(
        b"OPTIONS sip:example.com SIP/2.0\r\n\
          via: SIP/2.0/UDP 127.0.0.1:5060;rport;branch=z9hG4bK-replay\r\n\
          from: <sip:tooling@example.com>;tag=replay\r\n\
          to: <sip:example.com>\r\n\
          call-id: replay-1\r\n\
          cseq:   2 OPTIONS\r\n\
          max-forwards: 70\r\n\
          content-length: 0\r\n\
          \r\n",
    );

    client
        .send_message_raw(custom.clone(), server_addr)
        .await
        .expect("send_message_raw");

    let event = tokio::time::timeout(Duration::from_secs(2), server_rx.recv())
        .await
        .expect("server receive timeout")
        .expect("server channel closed");

    match event {
        TransportEvent::MessageReceived {
            raw_bytes: Some(bytes),
            ..
        } => {
            assert_eq!(
                bytes.as_ref(),
                custom.as_ref(),
                "send_message_raw must put the caller's bytes on the wire verbatim"
            );
        }
        other => panic!("expected MessageReceived with raw_bytes, got {:?}", other),
    }
}
