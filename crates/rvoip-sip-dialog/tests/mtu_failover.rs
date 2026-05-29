//! RFC 3261 §18.1.1 acceptance — `MultiplexedTransport` auto-fails
//! oversized UDP requests to TCP, and refuses to send when no TCP
//! transport is registered.
//!
//! Phase 10 of `STIR_SHAKEN_AND_PROXY_PLAN.md`. Motivated by Phase 2:
//! STIR/SHAKEN signing routinely adds a 1–2 KB `Identity:` header to
//! outbound INVITEs, which pushes them past UDP's safe size.

use async_trait::async_trait;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::identity::Identity;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{Message, Method, Request};

use rvoip_sip_dialog::transaction::transport::MultiplexedTransport;
use rvoip_sip_transport::{
    error::{Error as TransportError, Result as TransportResult},
    transport::TransportType,
    Transport,
};

/// Recording mock transport. Captures every `send_message` call so
/// tests can assert which transport actually received the request and
/// inspect the message it received (notably, the top Via's transport
/// after MTU-driven re-stamping).
#[derive(Debug)]
struct RecordingTransport {
    flavour: TransportType,
    addr: SocketAddr,
    sends: AtomicUsize,
    last_message: Mutex<Option<Message>>,
}

impl RecordingTransport {
    fn new(flavour: TransportType) -> Arc<Self> {
        Arc::new(Self {
            flavour,
            addr: "127.0.0.1:0".parse().unwrap(),
            sends: AtomicUsize::new(0),
            last_message: Mutex::new(None),
        })
    }

    fn count(&self) -> usize {
        self.sends.load(Ordering::SeqCst)
    }

    fn last_request(&self) -> Option<Request> {
        match self.last_message.lock().unwrap().clone()? {
            Message::Request(req) => Some(req),
            Message::Response(_) => None,
        }
    }
}

#[async_trait]
impl Transport for RecordingTransport {
    fn local_addr(&self) -> TransportResult<SocketAddr> {
        Ok(self.addr)
    }

    async fn send_message(
        &self,
        message: Message,
        _destination: SocketAddr,
    ) -> TransportResult<()> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        *self.last_message.lock().unwrap() = Some(message);
        Ok(())
    }

    async fn close(&self) -> TransportResult<()> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }

    fn supports_udp(&self) -> bool {
        self.flavour == TransportType::Udp
    }

    fn supports_tcp(&self) -> bool {
        self.flavour == TransportType::Tcp
    }

    fn default_transport_type(&self) -> TransportType {
        self.flavour
    }

    fn max_safe_message_size(&self) -> usize {
        match self.flavour {
            // Mirror the production UDP threshold (rvoip-sip-transport
            // exposes `UDP_SAFE_MAX_BYTES = 1300`).
            TransportType::Udp => 1300,
            _ => usize::MAX,
        }
    }
}

/// Build an INVITE addressed at a plain `sip:` URI (no `;transport=`),
/// so the multiplexer's URI-based selection lands on UDP by default.
fn base_invite() -> Request {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("mtu-failover-test")
        .cseq(1)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch("z9hG4bKmtu-test")],
            )
            .unwrap(),
        ))
        .build()
}

/// Build an INVITE padded past the UDP safe limit by a synthetic
/// PASSporT-shaped `Identity:` header — same shape Phase 2 produces.
fn oversized_signed_invite() -> Request {
    let mut request = base_invite();

    // Pad the JWT string so the serialized message comfortably exceeds
    // 1300 bytes. The exact JWT content doesn't matter for MTU policy —
    // only the serialized wire size does.
    let big_jwt = "x".repeat(3000);
    let identity = Identity::with_params(
        big_jwt,
        Some("https://cert.example.com/cert.pem".to_string()),
        Some("ES256".to_string()),
        Some("shaken".to_string()),
    );
    request.headers.push(TypedHeader::Identity(identity));
    request
}

fn registry(
    entries: Vec<(TransportType, Arc<dyn Transport>)>,
) -> HashMap<TransportType, Arc<dyn Transport>> {
    entries.into_iter().collect()
}

#[tokio::test]
async fn oversized_udp_request_fails_over_to_tcp() {
    let udp = RecordingTransport::new(TransportType::Udp);
    let tcp = RecordingTransport::new(TransportType::Tcp);

    let mux = MultiplexedTransport::new_without_trace(
        udp.clone() as Arc<dyn Transport>,
        registry(vec![
            (TransportType::Udp, udp.clone() as Arc<dyn Transport>),
            (TransportType::Tcp, tcp.clone() as Arc<dyn Transport>),
        ]),
    )
    .unwrap();

    let request = oversized_signed_invite();
    let wire_size = Message::Request(request.clone()).to_bytes().len();
    assert!(
        wire_size > 1300,
        "test fixture must exceed UDP safe limit; got {} bytes",
        wire_size
    );

    let dest: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    mux.send_message(Message::Request(request), dest)
        .await
        .expect("send_message should succeed via TCP failover");

    // The TCP transport must have received the send; UDP must not have.
    assert_eq!(
        tcp.count(),
        1,
        "TCP transport should receive the failover send"
    );
    assert_eq!(
        udp.count(),
        0,
        "UDP transport must be skipped when oversized"
    );

    // The TCP transport's recorded message must have its top Via flipped
    // to `SIP/2.0/TCP` so the peer routes the response back over TCP.
    let received = tcp
        .last_request()
        .expect("TCP must have a recorded request");
    let top_via = received
        .first_via()
        .expect("Via present on recorded request");
    let top_entry = top_via
        .headers()
        .first()
        .expect("Via has at least one entry");
    assert_eq!(
        top_entry.sent_protocol.transport, "TCP",
        "top Via sent-protocol must reflect the actual transport after failover"
    );
    // Branch must survive the flip — transaction key depends on it.
    assert_eq!(top_via.branch(), Some("z9hG4bKmtu-test"));
}

#[tokio::test]
async fn small_udp_request_stays_on_udp() {
    let udp = RecordingTransport::new(TransportType::Udp);
    let tcp = RecordingTransport::new(TransportType::Tcp);

    let mux = MultiplexedTransport::new_without_trace(
        udp.clone() as Arc<dyn Transport>,
        registry(vec![
            (TransportType::Udp, udp.clone() as Arc<dyn Transport>),
            (TransportType::Tcp, tcp.clone() as Arc<dyn Transport>),
        ]),
    )
    .unwrap();

    // Plain INVITE with no Identity header — well under 1300 bytes.
    let request = base_invite();
    let wire_size = Message::Request(request.clone()).to_bytes().len();
    assert!(
        wire_size < 1300,
        "small fixture must be under UDP safe limit; got {} bytes",
        wire_size
    );

    let dest: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    mux.send_message(Message::Request(request), dest)
        .await
        .unwrap();

    assert_eq!(udp.count(), 1, "small request must stay on UDP");
    assert_eq!(tcp.count(), 0, "TCP must not receive a small request");
}

#[tokio::test]
async fn oversized_udp_with_no_tcp_registered_is_message_too_large() {
    let udp = RecordingTransport::new(TransportType::Udp);

    // No TCP in the registry — only UDP.
    let mux = MultiplexedTransport::new_without_trace(
        udp.clone() as Arc<dyn Transport>,
        registry(vec![(
            TransportType::Udp,
            udp.clone() as Arc<dyn Transport>,
        )]),
    )
    .unwrap();

    let request = oversized_signed_invite();
    let wire_size = Message::Request(request.clone()).to_bytes().len();

    let dest: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let err = mux
        .send_message(Message::Request(request), dest)
        .await
        .expect_err("must fail closed when oversized UDP has no TCP fallback");

    match err {
        TransportError::MessageTooLarge(size) => {
            assert_eq!(
                size, wire_size,
                "MessageTooLarge should carry the serialized byte count"
            );
        }
        other => panic!("expected MessageTooLarge, got {:?}", other),
    }

    // UDP must NOT have been called — RFC §18.1.1 is MUST, not SHOULD.
    assert_eq!(
        udp.count(),
        0,
        "UDP must not be used when message exceeds safe size"
    );
}
