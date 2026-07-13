//! Phase 8.5 acceptance — hand-rolled stateless forwarder built on
//! `Transport::forward_raw_with_via_rewrite` + Via push/pop/detect_loop
//! primitives.
//!
//! Demonstrates that the helpers are sufficient to round-trip a SIP
//! INVITE through a byte-preserving stateless proxy without any
//! framework, AND that the RFC 8224 `Identity` header survives the
//! hop byte-for-byte.

use async_trait::async_trait;
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::identity::Identity;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{Message, Method};

use rvoip_sip_transport::error::Result as TransportResult;
use rvoip_sip_transport::transport::{apply_via_rewrite, ViaRewrite};
use rvoip_sip_transport::Transport;

/// Mock transport that records every raw byte buffer it would have
/// shipped. The forwarder tests rely on this to inspect the rewritten
/// wire form without round-tripping through a real socket.
#[derive(Debug, Default)]
struct CapturingTransport {
    sent_raw: Mutex<Vec<Bytes>>,
}

impl CapturingTransport {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn last_raw(&self) -> Option<Bytes> {
        self.sent_raw.lock().unwrap().last().cloned()
    }
}

#[async_trait]
impl Transport for CapturingTransport {
    fn local_addr(&self) -> TransportResult<SocketAddr> {
        Ok("127.0.0.1:5060".parse().unwrap())
    }

    async fn send_message(
        &self,
        _message: Message,
        _destination: SocketAddr,
    ) -> TransportResult<()> {
        // Not used in this test — forwarder uses send_message_raw.
        Ok(())
    }

    async fn send_message_raw(
        &self,
        bytes: Bytes,
        _destination: SocketAddr,
    ) -> TransportResult<()> {
        self.sent_raw.lock().unwrap().push(bytes);
        Ok(())
    }

    async fn close(&self) -> TransportResult<()> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

/// Build an INVITE whose typed `Identity:` header carries a fixed
/// JWT — so the test can assert the bytes survive end-to-end.
fn signed_invite_bytes(jwt: &str) -> Bytes {
    let req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@callee.example.com")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("tagA"))
        .to("Bob", "sip:bob@callee.example.com", None)
        .call_id("phase85-round-trip")
        .cseq(1)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch("z9hG4bKuac")],
            )
            .unwrap(),
        ))
        .header(TypedHeader::Identity(Identity::with_params(
            jwt,
            Some("https://cert.example.com/cert.pem".to_string()),
            Some("ES256".to_string()),
            Some("shaken".to_string()),
        )))
        .build();
    Bytes::from(Message::Request(req).to_bytes())
}

/// Synthesise the wire form of the proxy's own Via line for `Push`.
/// This is what a real stateless proxy would generate per RFC 3261
/// §16.11 (deterministic branch from inbound — here we use a fixed
/// test branch).
fn proxy_via_line(branch: &str) -> Bytes {
    Bytes::from(format!(
        "Via: SIP/2.0/UDP proxy.example.com:5060;branch={}\r\n",
        branch
    ))
}

#[tokio::test]
async fn request_forward_pushes_via_and_preserves_identity_bytes() {
    let jwt = "eyJhbGciOiJFUzI1NiJ9.eyJvcmlnIjp7InRuIjoiKzE1NTUxMjM0NTY3In19.signature";
    let inbound = signed_invite_bytes(jwt);

    // Sanity: Identity bytes are in the inbound wire form.
    assert!(
        inbound.windows(jwt.len()).any(|w| w == jwt.as_bytes()),
        "test fixture must contain the JWT in its wire form"
    );

    let downstream = CapturingTransport::new();
    let proxy_branch = "z9hG4bKproxy-det-001";
    let dest: SocketAddr = "192.0.2.10:5060".parse().unwrap();

    downstream
        .forward_raw_with_via_rewrite(
            inbound.clone(),
            ViaRewrite::Push(proxy_via_line(proxy_branch)),
            dest,
        )
        .await
        .expect("forward must succeed");

    let forwarded = downstream.last_raw().expect("a raw send was captured");

    // 1. Identity header bytes survive verbatim.
    assert!(
        forwarded.windows(jwt.len()).any(|w| w == jwt.as_bytes()),
        "JWT bytes must be preserved end-to-end"
    );

    // 2. The proxy's Via line is now on top.
    let forwarded_str = std::str::from_utf8(&forwarded).expect("UTF-8 wire form");
    let proxy_via_pos = forwarded_str
        .find("Via: SIP/2.0/UDP proxy.example.com")
        .expect("proxy Via inserted");
    let uac_via_pos = forwarded_str
        .find("Via: SIP/2.0/UDP 10.0.0.5")
        .expect("UAC Via preserved below proxy Via");
    assert!(
        proxy_via_pos < uac_via_pos,
        "proxy Via must appear above the original UAC Via"
    );

    // 3. The deterministic branch we chose appears exactly once.
    assert_eq!(
        forwarded_str.matches(proxy_branch).count(),
        1,
        "proxy branch should appear exactly once in the forwarded message"
    );

    // 4. The UAC's branch is untouched.
    assert!(forwarded_str.contains("branch=z9hG4bKuac"));
}

#[tokio::test]
async fn response_forward_pops_top_via_and_preserves_identity_bytes() {
    // Imagine the downstream UAS replies with a 200 OK that carries:
    //   - top Via = proxy's (we own it, must pop on the way back)
    //   - second Via = UAC's (the next-hop for our pop'd response)
    //   - the same Identity header we forwarded out (assertions about
    //     attestation, etc., can ride responses too in some profiles)
    let jwt = "eyJhbGciOiJFUzI1NiJ9.eyJvcmlnIjp7InRuIjoiKzE1NTUxMjM0NTY3In19.respSig";

    let response = rvoip_sip_core::builder::SimpleResponseBuilder::new(
        rvoip_sip_core::StatusCode::Ok,
        Some("OK"),
    )
    .from("Alice", "sip:alice@uac.example.com", Some("tagA"))
    .to("Bob", "sip:bob@callee.example.com", Some("tagB"))
    .call_id("phase85-round-trip")
    .cseq(1, Method::Invite)
    // Top Via = the proxy's (must be popped).
    .header(TypedHeader::Via(
        Via::new(
            "SIP",
            "2.0",
            "UDP",
            "proxy.example.com",
            Some(5060),
            vec![Param::branch("z9hG4bKproxy-det-001")],
        )
        .unwrap(),
    ))
    // Second Via = the UAC's — survives the pop.
    .header(TypedHeader::Via(
        Via::new(
            "SIP",
            "2.0",
            "UDP",
            "10.0.0.5",
            Some(5060),
            vec![Param::branch("z9hG4bKuac")],
        )
        .unwrap(),
    ))
    .header(TypedHeader::Identity(Identity::with_params(
        jwt,
        Some("https://cert.example.com/cert.pem".to_string()),
        Some("ES256".to_string()),
        Some("shaken".to_string()),
    )))
    .build();
    let inbound = Bytes::from(Message::Response(response).to_bytes());

    // Sanity: both Vias are present before the pop.
    let inbound_str = std::str::from_utf8(&inbound).unwrap();
    assert!(inbound_str.contains("proxy.example.com"));
    assert!(inbound_str.contains("10.0.0.5"));

    let downstream = CapturingTransport::new();
    let uac_addr: SocketAddr = "10.0.0.5:5060".parse().unwrap();

    downstream
        .forward_raw_with_via_rewrite(inbound, ViaRewrite::Pop, uac_addr)
        .await
        .expect("response pop must succeed");

    let forwarded = downstream.last_raw().expect("forwarded response captured");
    let forwarded_str = std::str::from_utf8(&forwarded).expect("UTF-8");

    // 1. Proxy's Via is GONE — its sent-by must not leak back to UAC.
    assert!(
        !forwarded_str.contains("proxy.example.com"),
        "proxy Via must be popped before the response continues upstream"
    );
    // 2. UAC's Via survives and is now the top.
    assert!(forwarded_str.contains("10.0.0.5"));
    // 3. Identity bytes survive end-to-end on the response too.
    assert!(
        forwarded.windows(jwt.len()).any(|w| w == jwt.as_bytes()),
        "JWT bytes must be preserved across the pop"
    );
}

#[tokio::test]
async fn forward_fails_loudly_when_no_via_present() {
    // A degenerate "request" with no Via header — apply_via_rewrite
    // must refuse rather than ship a malformed message.
    let bytes = Bytes::from_static(
        b"INVITE sip:bob@x SIP/2.0\r\n\
From: <sip:alice@x>;tag=a\r\n\
To: <sip:bob@x>\r\n\
Call-ID: no-via\r\n\
CSeq: 1 INVITE\r\n\
\r\n",
    );

    let err =
        apply_via_rewrite(bytes.clone(), ViaRewrite::Pop).expect_err("missing Via must error");
    assert!(matches!(
        err,
        rvoip_sip_transport::Error::ProtocolError(ref message)
            if message == "forward_raw_with_via_rewrite: message has no top Via header"
    ));

    let err2 = apply_via_rewrite(bytes, ViaRewrite::Push(Bytes::from_static(b"Via: x\r\n")))
        .expect_err("push without anchor Via must also error");
    assert!(matches!(
        err2,
        rvoip_sip_transport::Error::ProtocolError(ref message)
            if message == "forward_raw_with_via_rewrite: message has no top Via header"
    ));
}

#[test]
fn via_helpers_round_trip_at_the_typed_layer() {
    // The byte-level path above is the SHAKEN-preserving forwarder.
    // The typed-layer helpers are for stateful proxies that don't
    // need byte preservation — verify their composition works.

    let mut via = Via::new(
        "SIP",
        "2.0",
        "UDP",
        "10.0.0.5",
        Some(5060),
        vec![Param::branch("z9hG4bKuac")],
    )
    .unwrap();
    assert_eq!(via.headers().len(), 1);

    via.push_proxy_branch("TCP", "proxy.example.com", Some(5060), "z9hG4bKproxy")
        .expect("push succeeds");
    assert_eq!(via.headers().len(), 2);
    assert_eq!(via.headers()[0].sent_protocol.transport, "TCP");
    assert_eq!(via.headers()[0].sent_by_port, Some(5060));
    // Top branch is the proxy's — `Via::branch()` returns the FIRST.
    assert_eq!(via.branch(), Some("z9hG4bKproxy"));

    // Loop detection: a fabricated "previously stamped" Via with the
    // proxy's branch should trip the check.
    let stamped = Via::new(
        "SIP",
        "2.0",
        "UDP",
        "proxy.example.com",
        Some(5060),
        vec![Param::branch("z9hG4bKproxy")],
    )
    .unwrap();
    assert!(via.detect_loop(&[stamped]));

    // Unrelated branch — no loop.
    let unrelated = Via::new(
        "SIP",
        "2.0",
        "UDP",
        "other.example.com",
        None,
        vec![Param::branch("z9hG4bKunrelated")],
    )
    .unwrap();
    assert!(!via.detect_loop(&[unrelated]));

    // Pop puts us back to the original single UAC Via.
    let popped = via.pop_top().expect("pop yields the proxy entry");
    assert_eq!(popped.sent_protocol.transport, "TCP");
    assert_eq!(via.headers().len(), 1);
    assert_eq!(via.branch(), Some("z9hG4bKuac"));

    // Pop on empty Vec returns None safely.
    via.pop_top();
    assert!(via.pop_top().is_none());
}
