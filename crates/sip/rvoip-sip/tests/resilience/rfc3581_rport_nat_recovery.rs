//! RFC 3581 rport/NAT response routing and remaining binding-expiry coverage.

#[path = "support.rs"]
mod support;

use std::time::Duration;
use support::{document_stub, ResilienceLayer::LowerLibraryHardening, ResilienceStub};

#[tokio::test]
async fn rport_response_routing_survives_source_port_rewrite() {
    use rvoip_sip::{Config, UnifiedCoordinator};
    use tokio::net::UdpSocket;

    // Reserve a currently-free listener port, then hand it to rvoip-sip.
    let reservation = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let server_port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let server = UnifiedCoordinator::new(
        Config::local("rport-server", server_port).with_signaling_only_media(9),
    )
    .await
    .expect("start SIP server");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // This socket is the NAT-observed tuple. The Via deliberately advertises
    // an unrelated private tuple and asks the UAS to use RFC 3581 rport.
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let observed_client = client.local_addr().unwrap();
    let request = format!(
        "OPTIONS sip:rport-server@127.0.0.1:{server_port} SIP/2.0\r\n\
         Via: SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bK-rport-fixture;rport\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:nat-client@example.test>;tag=nat-source\r\n\
         To: <sip:rport-server@127.0.0.1>\r\n\
         Call-ID: rport-source-rewrite@example.test\r\n\
         CSeq: 1 OPTIONS\r\n\
         Contact: <sip:nat-client@10.0.0.5:5060>\r\n\
         Content-Length: 0\r\n\r\n"
    );
    client
        .send_to(request.as_bytes(), ("127.0.0.1", server_port))
        .await
        .expect("send rewritten-source request");

    let mut response = vec![0_u8; 4096];
    let (size, source) =
        tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut response))
            .await
            .expect("RFC3581 response timeout")
            .expect("receive RFC3581 response");
    let wire = String::from_utf8_lossy(&response[..size]);

    assert_eq!(source.port(), server_port);
    assert!(
        wire.starts_with("SIP/2.0 200"),
        "unexpected response: {wire}"
    );
    assert!(
        wire.contains("received=127.0.0.1"),
        "missing received: {wire}"
    );
    assert!(
        wire.contains(&format!("rport={}", observed_client.port())),
        "response did not echo observed source port: {wire}"
    );
    assert!(
        !wire.contains("rport=5060"),
        "response incorrectly routed to Via sent-by port: {wire}"
    );

    server
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("shutdown SIP server");
}

#[test]
#[ignore = "stub: requires NAT pinhole expiration simulation"]
fn nat_binding_refresh_or_failure_releases_dialog_cleanly() {
    document_stub(ResilienceStub {
        id: "RFC 3581 NAT binding resilience",
        layer: LowerLibraryHardening,
        existing_coverage: "No direct rvoip-sip NAT binding expiration test found.",
        target: "Expire a simulated NAT mapping mid-dialog and assert retry/timeout behavior produces one terminal event and no retained media/RTP/session resources.",
        next_hardening: "Add transport/proxy controls for source address changes and blackholing; rvoip-sip should only own terminal event and cleanup assertions.",
    });
}
