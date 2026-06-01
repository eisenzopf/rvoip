//! In-process STUN loopback test.
//!
//! Spins up a tiny mock STUN server (responds to Binding Requests
//! with a `XOR-MAPPED-ADDRESS` echoing the source address) and runs
//! `StunClient::discover` against it. Verifies the async client
//! round-trip end-to-end without any external dependency.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use rvoip_rtp_core::network::stun::{
    decode_binding_response, encode_binding_request, StunClient, MAGIC_COOKIE,
};
use tokio::net::UdpSocket;

/// Bind a UDP socket on 127.0.0.1 and return it plus its bound address.
async fn bind_local() -> (Arc<UdpSocket>, SocketAddr) {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    (Arc::new(sock), addr)
}

/// Run a one-shot STUN responder on `socket`. Reads one Binding
/// Request, replies with a Binding Response carrying a single
/// XOR-MAPPED-ADDRESS attribute set to the source address. Returns
/// when the response has been sent.
async fn run_responder(socket: UdpSocket) {
    let mut buf = [0u8; 1500];
    let (n, src) = socket.recv_from(&mut buf).await.unwrap();
    assert!(n >= 20, "request must include the 20-byte STUN header");

    // Echo back a synthetic response carrying source as XOR-MAPPED.
    let txn_id_slice = &buf[8..20];
    let txn_id: [u8; 12] = txn_id_slice.try_into().unwrap();

    let response = craft_xor_mapped_response(&txn_id, src);
    socket.send_to(&response, src).await.unwrap();
}

fn craft_xor_mapped_response(txn_id: &[u8; 12], addr: SocketAddr) -> Vec<u8> {
    const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
    const FAMILY_IPV4: u8 = 0x01;
    const BINDING_RESPONSE: u16 = 0x0101;

    let v4 = match addr.ip() {
        IpAddr::V4(v) => v,
        _ => panic!("test only supports IPv4 source"),
    };

    // XOR the address with the magic cookie + xport with high half of cookie.
    let xa = u32::from_be_bytes(v4.octets()) ^ MAGIC_COOKIE;
    let xport = addr.port() ^ ((MAGIC_COOKIE >> 16) as u16);

    let mut attr_body = Vec::new();
    attr_body.push(0);
    attr_body.push(FAMILY_IPV4);
    attr_body.extend_from_slice(&xport.to_be_bytes());
    attr_body.extend_from_slice(&xa.to_be_bytes());

    let attr_len = attr_body.len() as u16;
    let mut body = Vec::new();
    body.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
    body.extend_from_slice(&attr_len.to_be_bytes());
    body.extend_from_slice(&attr_body);

    let msg_len = body.len() as u16;
    let mut msg = Vec::new();
    msg.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
    msg.extend_from_slice(&msg_len.to_be_bytes());
    msg.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    msg.extend_from_slice(txn_id);
    msg.extend_from_slice(&body);
    msg
}

#[tokio::test]
async fn stun_client_round_trip_against_loopback_server() {
    // Server: bound, dedicated STUN responder.
    let server_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server_sock.local_addr().unwrap();
    let server = tokio::spawn(run_responder(server_sock));

    // Client: bind a separate socket. Send a probe at `server_addr`.
    // The server echoes our source addr back via XOR-MAPPED.
    let (client_sock, client_addr) = bind_local().await;
    let client = StunClient::new(client_sock, server_addr)
        .with_attempt_timeout(Duration::from_millis(500))
        .with_total_budget(Duration::from_millis(2_000));

    let discovered = client.discover().await.expect("STUN probe failed");
    assert_eq!(
        discovered, client_addr,
        "loopback responder should echo the client's bind address"
    );

    server.await.unwrap();
}

#[tokio::test]
async fn stun_client_times_out_when_server_silent() {
    // Bind a socket but don't run a responder against it.
    let (silent, silent_addr) = bind_local().await;
    drop(silent); // Free the port; sends to it will be dropped.

    // Use a different unbound port to avoid hitting an actual listener.
    let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), silent_addr.port());

    let (client_sock, _) = bind_local().await;
    let client = StunClient::new(client_sock, target)
        .with_attempt_timeout(Duration::from_millis(50))
        .with_total_budget(Duration::from_millis(150));

    let result = client.discover().await;
    assert!(
        matches!(
            result,
            Err(rvoip_rtp_core::network::stun::StunError::ProbeTimeout { .. })
        ),
        "expected ProbeTimeout, got {:?}",
        result
    );
}

#[tokio::test]
async fn encode_decode_round_trip_via_public_api() {
    let (request, txn) = encode_binding_request();
    // Manually craft a response with a known address.
    let echoed: SocketAddr = "203.0.113.99:54321".parse().unwrap();
    let response = craft_xor_mapped_response(&txn, echoed);
    let decoded = decode_binding_response(&response, &txn).unwrap();
    assert_eq!(decoded, echoed);
    // The request bytes are also valid (sanity check).
    assert_eq!(request.len(), 20);
}
