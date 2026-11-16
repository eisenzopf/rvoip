//! Proves the DTLS-SRTP handshake also completes when driven through the
//! demux bridge over a real `UdpRtpTransport` — not just over a directly
//! `connect()`-ed socket, which is all `dtls_srtp_handshake_test.rs` covers.
//!
//! A `UdpRtpTransport`'s socket is shared and *not* `connect()`-ed (it
//! receives from whatever peer sent a datagram, same as any real call's RTP
//! socket has to). `classify_rtp_mux_packet` already sorts DTLS bytes out
//! from RTP/RTCP on that shared socket; this test proves those bytes
//! actually reach a real handshake through `dtls_conn_adapter()` and that
//! two independent transports still derive matching keys end to end.

#![cfg(feature = "dtls-webrtc")]

use std::time::Duration;

use rvoip_rtp_core::dtls_srtp::{
    default_srtp_profiles, generate_identity, handshake_client, handshake_server,
};
use rvoip_rtp_core::transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport};

#[tokio::test]
async fn handshake_completes_through_the_shared_socket_demux_bridge() {
    // Bind to an address that is valid as a peer destination. The transport
    // default intentionally binds the wildcard address, whose `local_addr()`
    // is not a portable send target (notably on macOS).
    let config = || RtpTransportConfig {
        local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
        ..RtpTransportConfig::default()
    };
    let transport_a = UdpRtpTransport::new(config()).await.expect("transport A");
    let transport_b = UdpRtpTransport::new(config()).await.expect("transport B");

    let addr_a = transport_a
        .local_rtp_addr()
        .expect("transport A local addr");
    let addr_b = transport_b
        .local_rtp_addr()
        .expect("transport B local addr");

    // Same as a real call: each side learns its peer's RTP address from
    // SDP, then the shared socket is used for RTP, RTCP, *and* DTLS,
    // demultiplexed by the first byte of each datagram.
    transport_a.set_remote_rtp_addr(addr_b).await;
    transport_b.set_remote_rtp_addr(addr_a).await;

    let conn_a = transport_a
        .dtls_conn_adapter()
        .expect("transport A dtls conn adapter");
    let conn_b = transport_b
        .dtls_conn_adapter()
        .expect("transport B dtls conn adapter");

    let identity_a = generate_identity().unwrap();
    let identity_b = generate_identity().unwrap();

    let client_task = tokio::spawn(handshake_client(
        conn_a,
        identity_a,
        default_srtp_profiles(),
    ));
    let server_task = tokio::spawn(handshake_server(
        conn_b,
        identity_b,
        default_srtp_profiles(),
    ));

    let (client_result, server_result) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(client_task, server_task)
    })
    .await
    .expect("handshake over the demux bridge must complete within 10s");

    let client_result = client_result
        .expect("client task panicked")
        .expect("client handshake failed");
    let server_result = server_result
        .expect("server task panicked")
        .expect("server handshake failed");

    assert_eq!(client_result.profile, server_result.profile);
    assert_eq!(
        client_result.client_write_key.key(),
        server_result.client_write_key.key()
    );
    assert_eq!(
        client_result.server_write_key.key(),
        server_result.server_write_key.key()
    );
    assert_eq!(
        client_result.remote_fingerprint_sha256,
        server_result.local_fingerprint_sha256
    );
    assert_eq!(
        server_result.remote_fingerprint_sha256,
        client_result.local_fingerprint_sha256
    );
}

/// RFC 5764 §4.1 on the separate RTCP socket: without RTP/RTCP mux the RTCP
/// component runs its own handshake on its own 5-tuple. The peer here is a
/// plain connected UDP socket driving the DTLS engine directly, and its
/// handshake leaves from a NAT-mapped port on the RTP peer's IP rather than
/// the signalled RTCP address, so the transport must answer the source the
/// handshake actually arrives from. The derived keys then carry SRTCP on
/// that socket.
#[tokio::test]
async fn rtcp_component_handshake_runs_on_the_rtcp_socket_and_follows_a_nat_mapping() {
    use std::sync::Arc;

    use rvoip_rtp_core::packet::rtcp::{RtcpCompoundPacket, RtcpReceiverReport};
    use rvoip_rtp_core::srtp::SrtpContext;
    use rvoip_rtp_core::traits::RtpEvent;

    let transport = UdpRtpTransport::new(RtpTransportConfig {
        local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
        local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()),
        symmetric_rtp: true,
        rtcp_mux: false,
        session_id: Some("dtls-rtcp-component".to_string()),
        use_port_allocator: false,
        buffer_config: Default::default(),
    })
    .await
    .expect("transport");
    let local_rtcp = transport.local_rtcp_socket_addr().expect("RTCP socket");

    let peer_rtp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let signalled_rtcp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mapped_rtcp = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    transport
        .set_remote_rtp_addr(peer_rtp.local_addr().unwrap())
        .await;
    transport
        .set_remote_rtcp_addr(signalled_rtcp.local_addr().unwrap())
        .await;

    // Without the latch there is nothing to run the handshake for.
    assert!(transport.require_rtcp_component_keying());
    let conn = transport
        .dtls_rtcp_conn_adapter()
        .expect("RTCP DTLS adapter");

    mapped_rtcp.connect(local_rtcp).await.unwrap();
    let local_identity = generate_identity().unwrap();
    let local_fingerprint = local_identity.fingerprint_sha256;
    let server = tokio::spawn(handshake_server(
        conn,
        local_identity,
        default_srtp_profiles(),
    ));
    let client = tokio::spawn(handshake_client(
        Arc::clone(&mapped_rtcp) as _,
        generate_identity().unwrap(),
        default_srtp_profiles(),
    ));
    let (server, client) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(server, client)
    })
    .await
    .expect("the RTCP handshake must complete through the NAT mapping");
    let server = server.unwrap().expect("transport side of the handshake");
    let client = client.unwrap().expect("peer side of the handshake");
    assert_eq!(client.remote_fingerprint_sha256, local_fingerprint);
    assert_eq!(server.server_write_key.key(), client.server_write_key.key());

    let ctx = |key| SrtpContext::new(server.profile.clone(), key).unwrap();
    transport
        .install_rtcp_component_srtp_contexts(
            ctx(server.server_write_key.clone()),
            ctx(server.client_write_key.clone()),
        )
        .unwrap();
    let mut events = transport.subscribe();
    let report = RtcpCompoundPacket::new_with_rr(RtcpReceiverReport::new(0x0102_0304))
        .serialize()
        .unwrap();
    let mut peer_send = ctx(client.client_write_key.clone());
    mapped_rtcp
        .send(&peer_send.protect_rtcp(&report).unwrap())
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(RtpEvent::RtcpReceived { data, source })) => {
                assert_eq!(data, report);
                assert_eq!(source, mapped_rtcp.local_addr().unwrap());
                break;
            }
            Ok(Ok(_)) => {}
            other => panic!("SRTCP under the RTCP component's keys was not accepted: {other:?}"),
        }
    }
    transport.close().await.unwrap();
}
