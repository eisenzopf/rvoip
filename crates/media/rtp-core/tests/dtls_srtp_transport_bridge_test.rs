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
    let transport_a = UdpRtpTransport::new(RtpTransportConfig::default())
        .await
        .expect("transport A");
    let transport_b = UdpRtpTransport::new(RtpTransportConfig::default())
        .await
        .expect("transport B");

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
