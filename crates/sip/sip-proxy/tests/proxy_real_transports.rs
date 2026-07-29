//! Real-socket conformance evidence for the stateful proxy boundary.
//!
//! Unlike the proxy's deterministic mock-transport suites, these tests put
//! serialized SIP bytes through the shipping UDP and TCP transports, the real
//! transaction manager, and `StatefulProxy`. The peers are raw loopback
//! sockets so packet assertions observe precisely what crossed the boundary.

#[path = "support/real_transport.rs"]
mod real_transport;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use real_transport::{
    body, declared_content_length, header_values, request_wire, response_wire, start_line,
    udp_expect_quiet, udp_recv, udp_send, SipTcpPeer, TestCertificateFiles, IO_TIMEOUT,
};
use rvoip_sip_core::{parse_message, Message, Method, Request};
use rvoip_sip_dialog::transaction::TransactionManager;
use rvoip_sip_proxy::{ProxyConfig, RouteDecision, RouteFn, StatefulProxy};
use rvoip_sip_transport::transport::tls::{TlsClientConfig, TlsTransport};
use rvoip_sip_transport::transport::{TcpTransport, UdpTransport};
use rvoip_sip_transport::{Transport, TransportEvent, TransportRoute};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

struct RunningProxy {
    proxy: Arc<StatefulProxy>,
    transaction_manager: Arc<TransactionManager>,
    transport: Arc<dyn Transport>,
    task: JoinHandle<()>,
    local_addr: SocketAddr,
}

impl RunningProxy {
    async fn udp(target: SocketAddr) -> Self {
        let (transport, events) = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), None)
            .await
            .expect("bind proxy UDP transport");
        let local_addr = transport.local_addr().expect("proxy UDP address");
        Self::start(Arc::new(transport), events, local_addr, target).await
    }

    async fn tcp(target: SocketAddr) -> Self {
        let (transport, events) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(16), None)
                .await
                .expect("bind proxy TCP transport");
        let local_addr = transport.local_addr().expect("proxy TCP address");
        Self::start(Arc::new(transport), events, local_addr, target).await
    }

    async fn tls(
        target: SocketAddr,
        ca_certificate: &std::path::Path,
        certificate: &std::path::Path,
        private_key: &std::path::Path,
    ) -> Self {
        let (transport, events) = TlsTransport::bind_with_client_config(
            "127.0.0.1:0".parse().unwrap(),
            certificate,
            private_key,
            None,
            TlsClientConfig {
                extra_ca_path: Some(ca_certificate.to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("bind proxy TLS transport");
        let local_addr = transport.local_addr().expect("proxy TLS address");
        Self::start(Arc::new(transport), events, local_addr, target).await
    }

    async fn start(
        transport: Arc<dyn Transport>,
        events: mpsc::Receiver<TransportEvent>,
        local_addr: SocketAddr,
        target: SocketAddr,
    ) -> Self {
        let (transaction_manager, transaction_events) =
            TransactionManager::new(transport.clone(), events, Some(128))
                .await
                .expect("construct transaction manager");
        let transaction_manager = Arc::new(transaction_manager);
        let route: RouteFn = Arc::new(move |_request| Some(RouteDecision::to(target)));
        let proxy =
            StatefulProxy::with_config(transaction_manager.clone(), route, ProxyConfig::default());
        let task = proxy.clone().run(transaction_events);
        Self {
            proxy,
            transaction_manager,
            transport,
            task,
            local_addr,
        }
    }

    async fn shutdown(self) {
        self.task.abort();
        self.transport.close().await.expect("close proxy transport");
        // These fields deliberately keep the complete live boundary alive
        // until the socket assertions have finished.
        drop(self.proxy);
        drop(self.transaction_manager);
    }
}

struct ReceivedTransportMessage {
    wire: Vec<u8>,
    message: Message,
    response_route: TransportRoute,
}

async fn recv_transport_matching<F>(
    events: &mut mpsc::Receiver<TransportEvent>,
    mut predicate: F,
) -> ReceivedTransportMessage
where
    F: FnMut(&[u8]) -> bool,
{
    let deadline = Instant::now() + IO_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "matching transport SIP message timed out"
        );
        let event = tokio::time::timeout(remaining, events.recv())
            .await
            .expect("matching transport SIP message timed out")
            .expect("transport event channel closed");
        if let TransportEvent::MessageReceived {
            message,
            source,
            transport_type,
            flow_id,
            raw_bytes,
            ..
        } = event
        {
            let wire = raw_bytes
                .expect("shipping transport must expose serialized wire bytes")
                .to_vec();
            if !predicate(&wire) {
                continue;
            }
            let mut response_route =
                TransportRoute::new(source).with_transport_type(transport_type);
            if let Some(flow_id) = flow_id {
                response_route = response_route.with_flow_id(flow_id);
            }
            return ReceivedTransportMessage {
                wire,
                message,
                response_route,
            };
        }
    }
}

async fn udp_recv_matching<F>(socket: &UdpSocket, mut predicate: F) -> (Vec<u8>, SocketAddr)
where
    F: FnMut(&[u8]) -> bool,
{
    let deadline = Instant::now() + IO_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "matching UDP SIP packet timed out");
        let packet = tokio::time::timeout(remaining, udp_recv(socket))
            .await
            .expect("matching UDP SIP packet timed out");
        if predicate(&packet.0) {
            return packet;
        }
    }
}

async fn tcp_read_matching<F>(peer: &mut SipTcpPeer, mut predicate: F) -> Vec<u8>
where
    F: FnMut(&[u8]) -> bool,
{
    let deadline = Instant::now() + IO_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "matching TCP SIP message timed out");
        let message = tokio::time::timeout(remaining, peer.read_message())
            .await
            .expect("matching TCP SIP message timed out");
        if predicate(&message) {
            return message;
        }
    }
}

fn is_request(message: &[u8], method: &str) -> bool {
    start_line(message).starts_with(&format!("{method} "))
}

fn is_response(message: &[u8], status: u16, method: Method) -> bool {
    let Ok(Message::Response(response)) = parse_message(message) else {
        return false;
    };
    response.status().as_u16() == status
        && response.cseq().is_some_and(|cseq| cseq.method() == &method)
}

fn assert_body_and_length(message: &[u8], expected: &[u8]) {
    assert_eq!(body(message), expected, "wire body changed");
    assert_eq!(
        declared_content_length(message),
        expected.len(),
        "Content-Length no longer describes the wire body"
    );
}

fn parsed_request(message: &[u8]) -> Request {
    let Message::Request(request) = parse_message(message).expect("parse SIP request") else {
        panic!("expected SIP request");
    };
    request
}

fn assert_generated_cancel_matches_invite(invite_wire: &[u8], cancel_wire: &[u8]) {
    let invite = parsed_request(invite_wire);
    let cancel = parsed_request(cancel_wire);
    assert_eq!(cancel.method(), Method::Cancel);
    assert_eq!(
        cancel.uri(),
        invite.uri(),
        "generated CANCEL Request-URI changed"
    );
    assert_eq!(
        cancel.call_id().map(ToString::to_string),
        invite.call_id().map(ToString::to_string),
        "generated CANCEL Call-ID changed"
    );
    assert_eq!(
        cancel.from().map(ToString::to_string),
        invite.from().map(ToString::to_string),
        "generated CANCEL From changed"
    );
    assert_eq!(
        cancel.to().map(ToString::to_string),
        invite.to().map(ToString::to_string),
        "generated CANCEL To changed"
    );
    let invite_cseq = invite.cseq().expect("downstream INVITE CSeq");
    let cancel_cseq = cancel.cseq().expect("generated CANCEL CSeq");
    assert_eq!(
        cancel_cseq.sequence(),
        invite_cseq.sequence(),
        "generated CANCEL numeric CSeq changed"
    );
    assert_eq!(cancel_cseq.method(), &Method::Cancel);
    assert_eq!(
        cancel.via_headers().len(),
        1,
        "generated CANCEL must carry only the copied top INVITE Via"
    );
    assert_eq!(
        cancel.first_via().map(|via| via.to_string()),
        invite.first_via().map(|via| via.to_string()),
        "generated CANCEL top Via changed"
    );
    assert_eq!(
        header_values(cancel_wire, "route"),
        header_values(invite_wire, "route"),
        "generated CANCEL Route set changed"
    );
    assert_body_and_length(cancel_wire, b"");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_live_proxy_pushes_and_pops_one_via_and_preserves_bodies() {
    let uac = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP UAC");
    let uas = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP UAS");
    let proxy = RunningProxy::udp(uas.local_addr().unwrap()).await;
    let request_body = b"v=0\r\no=alice 1 1 IN IP4 127.0.0.1\r\ns=wire-evidence\r\n";
    let invite = request_wire(
        "INVITE",
        "sip:bob@example.test",
        "UDP",
        uac.local_addr().unwrap(),
        "z9hG4bK-real-udp-body",
        "real-udp-body@example.test",
        1,
        70,
        request_body,
    );

    udp_send(&uac, &invite, proxy.local_addr).await;
    let (forwarded, source) =
        udp_recv_matching(&uas, |message| is_request(message, "INVITE")).await;
    assert_eq!(source, proxy.local_addr);
    let Message::Request(parsed_forwarded) =
        parse_message(&forwarded).expect("parse forwarded UDP INVITE")
    else {
        panic!("expected forwarded request");
    };
    let vias = parsed_forwarded.via_headers();
    assert_eq!(vias.len(), 2, "proxy must push exactly one Via");
    assert_eq!(vias[0].headers()[0].transport(), "UDP");
    assert_eq!(
        vias[1].branch(),
        Some("z9hG4bK-real-udp-body"),
        "upstream Via branch changed"
    );
    assert_body_and_length(&forwarded, request_body);

    let response_body = b"downstream failure detail";
    let response = response_wire(
        &forwarded,
        488,
        "Not Acceptable Here",
        "uas-udp-tag",
        response_body,
    );
    udp_send(&uas, &response, proxy.local_addr).await;
    let (upstream, _) =
        udp_recv_matching(&uac, |message| is_response(message, 488, Method::Invite)).await;
    let Message::Response(parsed_upstream) =
        parse_message(&upstream).expect("parse forwarded UDP response")
    else {
        panic!("expected forwarded response");
    };
    let vias = parsed_upstream.via_headers();
    assert_eq!(vias.len(), 1, "proxy must pop exactly its own Via");
    assert_eq!(vias[0].branch(), Some("z9hG4bK-real-udp-body"));
    assert_body_and_length(&upstream, response_body);

    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_response_without_rport_uses_via_sent_by_port_not_packet_source_port() {
    let ingress_source = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind NAT-like UDP source");
    let via_destination = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind advertised Via destination");
    let uas = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP UAS");
    let proxy = RunningProxy::udp(uas.local_addr().unwrap()).await;
    let request = request_wire(
        "OPTIONS",
        "sip:probe@example.test",
        "UDP",
        via_destination.local_addr().unwrap(),
        "z9hG4bK-real-via-destination",
        "real-via-destination@example.test",
        1,
        70,
        b"",
    );
    let request = String::from_utf8(request)
        .expect("generated request is ASCII")
        .replace(";rport\r\n", "\r\n")
        .into_bytes();

    udp_send(&ingress_source, &request, proxy.local_addr).await;
    let (forwarded, _) = udp_recv_matching(&uas, |message| is_request(message, "OPTIONS")).await;
    let response = response_wire(&forwarded, 200, "OK", "uas-via-destination-tag", b"");
    udp_send(&uas, &response, proxy.local_addr).await;

    let (upstream, source) = udp_recv_matching(&via_destination, |message| {
        is_response(message, 200, Method::Options)
    })
    .await;
    assert_eq!(source, proxy.local_addr);
    let Message::Response(parsed) =
        parse_message(&upstream).expect("parse response delivered to Via destination")
    else {
        panic!("expected SIP response");
    };
    assert_eq!(parsed.via_headers().len(), 1);
    assert_eq!(
        parsed.via_headers()[0].headers()[0].port(),
        Some(via_destination.local_addr().unwrap().port())
    );
    udp_expect_quiet(&ingress_source, Duration::from_millis(150)).await;

    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_live_local_final_retransmission_reuses_the_exact_to_tag() {
    let uac = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP UAC");
    let sink = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind unused UDP target");
    let proxy = RunningProxy::udp(sink.local_addr().unwrap()).await;
    let invite = request_wire(
        "INVITE",
        "sip:bob@example.test",
        "UDP",
        uac.local_addr().unwrap(),
        "z9hG4bK-real-local-final",
        "real-local-final@example.test",
        1,
        0,
        b"",
    );

    udp_send(&uac, &invite, proxy.local_addr).await;
    let (first, _) =
        udp_recv_matching(&uac, |message| is_response(message, 483, Method::Invite)).await;
    udp_send(&uac, &invite, proxy.local_addr).await;
    let (retransmitted, _) =
        udp_recv_matching(&uac, |message| is_response(message, 483, Method::Invite)).await;

    let first_to = header_values(&first, "to");
    let retransmitted_to = header_values(&retransmitted, "to");
    assert_eq!(first_to.len(), 1);
    assert_eq!(first_to, retransmitted_to);
    assert!(
        first_to[0].contains(";tag=rvoip-proxy-"),
        "local final response did not add the proxy's stable To tag: {}",
        first_to[0]
    );
    udp_expect_quiet(&sink, Duration::from_millis(150)).await;

    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_live_matched_cancel_completes_upstream_and_cancels_proceeding_leg() {
    let uac = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP UAC");
    let uas = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP UAS");
    let proxy = RunningProxy::udp(uas.local_addr().unwrap()).await;
    let branch = "z9hG4bK-real-matched-cancel";
    let call_id = "real-matched-cancel@example.test";
    let invite = request_wire(
        "INVITE",
        "sip:bob@example.test",
        "UDP",
        uac.local_addr().unwrap(),
        branch,
        call_id,
        7,
        70,
        b"",
    );

    udp_send(&uac, &invite, proxy.local_addr).await;
    let (forwarded_invite, invite_source) =
        udp_recv_matching(&uas, |message| is_request(message, "INVITE")).await;
    let ringing = response_wire(&forwarded_invite, 180, "Ringing", "uas-cancel-tag", b"");
    udp_send(&uas, &ringing, proxy.local_addr).await;
    udp_recv_matching(&uac, |message| is_response(message, 180, Method::Invite)).await;

    let cancel = request_wire(
        "CANCEL",
        "sip:bob@example.test",
        "UDP",
        uac.local_addr().unwrap(),
        branch,
        call_id,
        7,
        70,
        b"",
    );
    udp_send(&uac, &cancel, proxy.local_addr).await;
    let (cancel_ok, _) =
        udp_recv_matching(&uac, |message| is_response(message, 200, Method::Cancel)).await;
    assert!(
        header_values(&cancel_ok, "to")
            .first()
            .is_some_and(|value| value.contains(";tag=")),
        "matched CANCEL's local 200 must have a To tag"
    );

    let (downstream_cancel, cancel_source) =
        udp_recv_matching(&uas, |message| is_request(message, "CANCEL")).await;
    assert_eq!(
        cancel_source, invite_source,
        "UDP generated CANCEL must use the INVITE's exact next hop"
    );
    assert_generated_cancel_matches_invite(&forwarded_invite, &downstream_cancel);

    let cancel_response = response_wire(&downstream_cancel, 200, "OK", "uas-cancel-tag", b"");
    udp_send(&uas, &cancel_response, proxy.local_addr).await;
    let invite_final = response_wire(
        &forwarded_invite,
        487,
        "Request Terminated",
        "uas-cancel-tag",
        b"",
    );
    udp_send(&uas, &invite_final, proxy.local_addr).await;
    udp_recv_matching(&uac, |message| is_response(message, 487, Method::Invite)).await;

    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_live_unmatched_cancel_is_stateless_and_true_stray_response_is_dropped() {
    let uac = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP UAC");
    let uas = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP UAS");
    let proxy = RunningProxy::udp(uas.local_addr().unwrap()).await;
    let cancel = request_wire(
        "CANCEL",
        "sip:bob@example.test",
        "UDP",
        uac.local_addr().unwrap(),
        "z9hG4bK-real-unmatched-cancel",
        "real-unmatched-cancel@example.test",
        9,
        70,
        b"",
    );

    udp_send(&uac, &cancel, proxy.local_addr).await;
    let (forwarded_cancel, _) =
        udp_recv_matching(&uas, |message| is_request(message, "CANCEL")).await;
    let ok = response_wire(&forwarded_cancel, 200, "OK", "uas-unmatched-tag", b"");
    udp_send(&uas, &ok, proxy.local_addr).await;
    let (forwarded_ok, _) =
        udp_recv_matching(&uac, |message| is_response(message, 200, Method::Cancel)).await;
    let Message::Response(parsed_ok) = parse_message(&forwarded_ok).expect("parse CANCEL 200")
    else {
        panic!("expected CANCEL response");
    };
    assert_eq!(parsed_ok.via_headers().len(), 1);
    assert_eq!(
        parsed_ok.via_headers()[0].branch(),
        Some("z9hG4bK-real-unmatched-cancel")
    );

    let stray_request = request_wire(
        "OPTIONS",
        "sip:bob@example.test",
        "UDP",
        proxy.local_addr,
        "z9hG4bK-proxy-never-registered",
        "real-stray-response@example.test",
        11,
        70,
        b"",
    );
    let stray_response = response_wire(&stray_request, 200, "OK", "stray-tag", b"");
    udp_send(&uas, &stray_response, proxy.local_addr).await;
    udp_expect_quiet(&uac, Duration::from_millis(250)).await;

    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcp_live_proxy_uses_ingress_flow_and_preserves_framed_bodies() {
    let uas_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind TCP UAS");
    let proxy = RunningProxy::tcp(uas_listener.local_addr().unwrap()).await;
    let mut uac = SipTcpPeer::connect(proxy.local_addr).await;
    let uac_sent_by: SocketAddr = "127.0.0.1:5099".parse().unwrap();
    let request_body = b"v=0\r\ns=tcp-wire-evidence\r\n";
    let invite = request_wire(
        "INVITE",
        "sip:bob@example.test;transport=tcp",
        "TCP",
        uac_sent_by,
        "z9hG4bK-real-tcp-body",
        "real-tcp-body@example.test",
        1,
        70,
        request_body,
    );
    uac.write_message(&invite).await;

    let (downstream_stream, downstream_peer) =
        tokio::time::timeout(IO_TIMEOUT, uas_listener.accept())
            .await
            .expect("proxy TCP downstream connect timeout")
            .expect("accept proxy TCP downstream");
    assert_eq!(downstream_peer.ip(), proxy.local_addr.ip());
    let mut uas = SipTcpPeer::new(downstream_stream);
    let forwarded = tcp_read_matching(&mut uas, |message| is_request(message, "INVITE")).await;
    let Message::Request(parsed_forwarded) =
        parse_message(&forwarded).expect("parse forwarded TCP INVITE")
    else {
        panic!("expected forwarded TCP request");
    };
    let vias = parsed_forwarded.via_headers();
    assert_eq!(vias.len(), 2);
    assert_eq!(vias[0].headers()[0].transport(), "TCP");
    assert_eq!(
        vias[0].headers()[0].sent_by_port,
        Some(proxy.local_addr.port())
    );
    assert_eq!(vias[1].branch(), Some("z9hG4bK-real-tcp-body"));
    assert_body_and_length(&forwarded, request_body);

    let quiet_window = proxy
        .transaction_manager
        .timer_settings()
        .t1
        .saturating_mul(2);
    if let Ok(unexpected) = tokio::time::timeout(quiet_window, uas.read_message()).await {
        panic!(
            "reliable TCP transaction emitted another SIP message within two T1 intervals: {}",
            start_line(&unexpected)
        );
    }

    let response_body = b"tcp response body";
    let response = response_wire(&forwarded, 486, "Busy Here", "uas-tcp-tag", response_body);
    uas.write_message(&response).await;
    let upstream = tcp_read_matching(&mut uac, |message| {
        is_response(message, 486, Method::Invite)
    })
    .await;
    let Message::Response(parsed_upstream) =
        parse_message(&upstream).expect("parse forwarded TCP response")
    else {
        panic!("expected forwarded TCP response");
    };
    assert_eq!(parsed_upstream.via_headers().len(), 1);
    assert_eq!(
        parsed_upstream.via_headers()[0].branch(),
        Some("z9hG4bK-real-tcp-body")
    );
    assert_body_and_length(&upstream, response_body);

    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcp_live_generated_cancel_copies_invite_and_reuses_the_exact_connection() {
    let uas_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind TCP UAS");
    let proxy = RunningProxy::tcp(uas_listener.local_addr().unwrap()).await;
    let mut uac = SipTcpPeer::connect(proxy.local_addr).await;
    let uac_sent_by: SocketAddr = "127.0.0.1:5097".parse().unwrap();
    let branch = "z9hG4bK-real-tcp-cancel";
    let call_id = "real-tcp-cancel@example.test";
    let invite = request_wire(
        "INVITE",
        "sip:bob@example.test;transport=tcp",
        "TCP",
        uac_sent_by,
        branch,
        call_id,
        17,
        70,
        b"",
    );
    uac.write_message(&invite).await;

    let (downstream_stream, _) = tokio::time::timeout(IO_TIMEOUT, uas_listener.accept())
        .await
        .expect("proxy TCP downstream connect timeout")
        .expect("accept proxy TCP downstream");
    let mut uas = SipTcpPeer::new(downstream_stream);
    let forwarded_invite =
        tcp_read_matching(&mut uas, |message| is_request(message, "INVITE")).await;
    let ringing = response_wire(&forwarded_invite, 180, "Ringing", "uas-tcp-cancel-tag", b"");
    uas.write_message(&ringing).await;
    tcp_read_matching(&mut uac, |message| {
        is_response(message, 180, Method::Invite)
    })
    .await;

    let cancel = request_wire(
        "CANCEL",
        "sip:bob@example.test;transport=tcp",
        "TCP",
        uac_sent_by,
        branch,
        call_id,
        17,
        70,
        b"",
    );
    uac.write_message(&cancel).await;
    tcp_read_matching(&mut uac, |message| {
        is_response(message, 200, Method::Cancel)
    })
    .await;
    let downstream_cancel =
        tcp_read_matching(&mut uas, |message| is_request(message, "CANCEL")).await;
    assert_generated_cancel_matches_invite(&forwarded_invite, &downstream_cancel);

    let cancel_ok = response_wire(&downstream_cancel, 200, "OK", "uas-tcp-cancel-tag", b"");
    uas.write_message(&cancel_ok).await;
    let invite_final = response_wire(
        &forwarded_invite,
        487,
        "Request Terminated",
        "uas-tcp-cancel-tag",
        b"",
    );
    uas.write_message(&invite_final).await;
    tcp_read_matching(&mut uac, |message| {
        is_response(message, 487, Method::Invite)
    })
    .await;

    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sips_live_proxy_uses_verified_tls_both_ways_and_exact_response_flows() {
    let certificates = TestCertificateFiles::create();
    let trusted_client = || TlsClientConfig {
        extra_ca_path: Some(certificates.ca_certificate().to_owned()),
        ..Default::default()
    };

    let (uas, mut uas_events) = TlsTransport::bind_with_client_config(
        "127.0.0.1:0".parse().unwrap(),
        certificates.certificate(),
        certificates.private_key(),
        None,
        trusted_client(),
    )
    .await
    .expect("bind TLS UAS");
    let uas = Arc::new(uas);
    let uas_addr = uas.local_addr().expect("TLS UAS address");
    let proxy = RunningProxy::tls(
        uas_addr,
        certificates.ca_certificate(),
        certificates.certificate(),
        certificates.private_key(),
    )
    .await;
    let (uac, mut uac_events) =
        TlsTransport::client_only("127.0.0.1:5098".parse().unwrap(), None, trusted_client())
            .await
            .expect("create TLS UAC");
    let uac = Arc::new(uac);

    let request_body = b"v=0\r\ns=sips-wire-evidence\r\n";
    let invite = request_wire(
        "INVITE",
        &format!("sips:bob@localhost:{}", uas_addr.port()),
        "TLS",
        "127.0.0.1:5098".parse().unwrap(),
        "z9hG4bK-real-sips-body",
        "real-sips-body@example.test",
        1,
        70,
        request_body,
    );
    let parsed_invite = parse_message(&invite).expect("parse SIPS INVITE");
    uac.send_message(parsed_invite, proxy.local_addr)
        .await
        .expect("send verified TLS request to proxy");

    let downstream =
        recv_transport_matching(&mut uas_events, |wire| is_request(wire, "INVITE")).await;
    let Message::Request(parsed_forwarded) = &downstream.message else {
        panic!("expected SIPS request");
    };
    let vias = parsed_forwarded.via_headers();
    assert_eq!(vias.len(), 2);
    assert_eq!(vias[0].headers()[0].transport(), "TLS");
    assert_eq!(vias[1].branch(), Some("z9hG4bK-real-sips-body"));
    assert_body_and_length(&downstream.wire, request_body);
    assert!(
        downstream.response_route.flow_id.is_some(),
        "TLS ingress must identify the exact authenticated flow"
    );

    let response_body = b"sips response body";
    let response = response_wire(
        &downstream.wire,
        486,
        "Busy Here",
        "uas-sips-tag",
        response_body,
    );
    uas.send_message_via(
        parse_message(&response).expect("parse SIPS response"),
        downstream.response_route,
    )
    .await
    .expect("return SIPS response on exact ingress flow");

    let upstream = recv_transport_matching(&mut uac_events, |wire| {
        is_response(wire, 486, Method::Invite)
    })
    .await;
    let Message::Response(parsed_upstream) = upstream.message else {
        panic!("expected SIPS response");
    };
    assert_eq!(parsed_upstream.via_headers().len(), 1);
    assert_eq!(
        parsed_upstream.via_headers()[0].branch(),
        Some("z9hG4bK-real-sips-body")
    );
    assert_body_and_length(&upstream.wire, response_body);

    uac.close().await.expect("close TLS UAC");
    uas.close().await.expect("close TLS UAS");
    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sips_live_generated_cancel_copies_invite_and_reuses_the_exact_tls_flow() {
    let certificates = TestCertificateFiles::create();
    let trusted_client = || TlsClientConfig {
        extra_ca_path: Some(certificates.ca_certificate().to_owned()),
        ..Default::default()
    };

    let (uas, mut uas_events) = TlsTransport::bind_with_client_config(
        "127.0.0.1:0".parse().unwrap(),
        certificates.certificate(),
        certificates.private_key(),
        None,
        trusted_client(),
    )
    .await
    .expect("bind TLS UAS");
    let uas = Arc::new(uas);
    let uas_addr = uas.local_addr().expect("TLS UAS address");
    let proxy = RunningProxy::tls(
        uas_addr,
        certificates.ca_certificate(),
        certificates.certificate(),
        certificates.private_key(),
    )
    .await;
    let uac_addr: SocketAddr = "127.0.0.1:5096".parse().unwrap();
    let (uac, mut uac_events) = TlsTransport::client_only(uac_addr, None, trusted_client())
        .await
        .expect("create TLS UAC");
    let uac = Arc::new(uac);
    let branch = "z9hG4bK-real-sips-cancel";
    let call_id = "real-sips-cancel@example.test";
    let request_uri = format!("sips:bob@localhost:{}", uas_addr.port());
    let invite = request_wire(
        "INVITE",
        &request_uri,
        "TLS",
        uac_addr,
        branch,
        call_id,
        19,
        70,
        b"",
    );
    uac.send_message(
        parse_message(&invite).expect("parse SIPS CANCEL-test INVITE"),
        proxy.local_addr,
    )
    .await
    .expect("send verified TLS INVITE to proxy");

    let downstream_invite =
        recv_transport_matching(&mut uas_events, |wire| is_request(wire, "INVITE")).await;
    let ringing = response_wire(
        &downstream_invite.wire,
        180,
        "Ringing",
        "uas-sips-cancel-tag",
        b"",
    );
    uas.send_message_via(
        parse_message(&ringing).expect("parse SIPS 180"),
        downstream_invite.response_route.clone(),
    )
    .await
    .expect("return SIPS 180 on exact ingress flow");
    recv_transport_matching(&mut uac_events, |wire| {
        is_response(wire, 180, Method::Invite)
    })
    .await;

    let cancel = request_wire(
        "CANCEL",
        &request_uri,
        "TLS",
        uac_addr,
        branch,
        call_id,
        19,
        70,
        b"",
    );
    uac.send_message(
        parse_message(&cancel).expect("parse SIPS CANCEL"),
        proxy.local_addr,
    )
    .await
    .expect("send SIPS CANCEL to proxy");
    recv_transport_matching(&mut uac_events, |wire| {
        is_response(wire, 200, Method::Cancel)
    })
    .await;
    let downstream_cancel =
        recv_transport_matching(&mut uas_events, |wire| is_request(wire, "CANCEL")).await;
    assert_eq!(
        downstream_cancel.response_route.flow_id, downstream_invite.response_route.flow_id,
        "generated CANCEL must reuse the INVITE's exact TLS connection"
    );
    assert_generated_cancel_matches_invite(&downstream_invite.wire, &downstream_cancel.wire);

    let cancel_ok = response_wire(
        &downstream_cancel.wire,
        200,
        "OK",
        "uas-sips-cancel-tag",
        b"",
    );
    uas.send_message_via(
        parse_message(&cancel_ok).expect("parse SIPS CANCEL 200"),
        downstream_cancel.response_route,
    )
    .await
    .expect("return SIPS CANCEL response on exact flow");
    let invite_final = response_wire(
        &downstream_invite.wire,
        487,
        "Request Terminated",
        "uas-sips-cancel-tag",
        b"",
    );
    uas.send_message_via(
        parse_message(&invite_final).expect("parse SIPS INVITE 487"),
        downstream_invite.response_route,
    )
    .await
    .expect("return SIPS INVITE final on exact flow");
    recv_transport_matching(&mut uac_events, |wire| {
        is_response(wire, 487, Method::Invite)
    })
    .await;

    uac.close().await.expect("close TLS UAC");
    uas.close().await.expect("close TLS UAS");
    proxy.shutdown().await;
}
