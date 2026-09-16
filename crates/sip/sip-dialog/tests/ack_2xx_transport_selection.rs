//! A UAC INVITE sent over TCP through a proxy, answered by a 2xx whose
//! Record-Route decides where and how the ACK travels (RFC 3261 §13.2.2.4,
//! §8.1.2, RFC 3263 §4.1). The proxy is a raw socket pair so every wire write
//! is observable.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_dialog::transaction::transport::MultiplexedTransport;
use rvoip_sip_dialog::transaction::{TransactionEvent, TransactionManager};
use rvoip_sip_transport::resolver::{ResolvedTarget, Resolver, ResolverError};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{
    TcpTransport, Transport, TransportAuthority, TransportEvent, TransportRoute, UdpTransport,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;

const PROXY_HOST: &str = "proxy.ack-test.invalid";

struct FixedResolver(ResolvedTarget);

#[async_trait]
impl Resolver for FixedResolver {
    async fn resolve(&self, _uri: &Uri) -> std::result::Result<Vec<ResolvedTarget>, ResolverError> {
        Ok(vec![self.0.clone()])
    }
}

struct Uac {
    manager: Arc<TransactionManager>,
    events: mpsc::Receiver<TransactionEvent>,
    local: SocketAddr,
}

async fn uac() -> Uac {
    let (udp, mut udp_rx) = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(64))
        .await
        .expect("bind UDP");
    let local = udp.local_addr().expect("UDP address");
    let (tcp, mut tcp_rx) = TcpTransport::bind(local, Some(64), None)
        .await
        .expect("bind TCP");
    let udp: Arc<dyn Transport> = Arc::new(udp);
    let tcp: Arc<dyn Transport> = Arc::new(tcp);
    let transports = HashMap::from([(TransportType::Udp, udp.clone()), (TransportType::Tcp, tcp)]);
    let mux = MultiplexedTransport::new_without_trace(udp, transports).expect("multiplexer");

    let (events_tx, events_rx) = mpsc::channel::<TransportEvent>(256);
    let tcp_events_tx = events_tx.clone();
    tokio::spawn(async move {
        while let Some(event) = udp_rx.recv().await {
            let _ = events_tx.send(event).await;
        }
    });
    tokio::spawn(async move {
        while let Some(event) = tcp_rx.recv().await {
            let _ = tcp_events_tx.send(event).await;
        }
    });

    let (manager, events) = TransactionManager::new(Arc::new(mux), events_rx, Some(64))
        .await
        .expect("transaction manager");
    Uac {
        manager: Arc::new(manager),
        events,
        local,
    }
}

struct Proxy {
    tcp: TcpListener,
    udp: UdpSocket,
    addr: SocketAddr,
}

async fn proxy() -> Proxy {
    let tcp = TcpListener::bind("127.0.0.1:0").await.expect("proxy TCP");
    let addr = tcp.local_addr().expect("proxy address");
    let udp = UdpSocket::bind(addr).await.expect("proxy UDP");
    Proxy { tcp, udp, addr }
}

async fn send_invite(uac: &Uac, proxy: SocketAddr, call_id: &str) {
    let mut invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.test")
        .unwrap()
        .from("Alice", "sip:alice@example.test", Some("alice-tag"))
        .to("Bob", "sip:bob@example.test", None)
        .contact(&format!("sip:alice@{};transport=tcp", uac.local), None)
        .call_id(call_id)
        .cseq(1)
        .via(&uac.local.to_string(), "TCP", Some("z9hG4bK.ack-transport"))
        .max_forwards(70)
        .build();
    invite
        .headers
        .push(TypedHeader::Route(Route::with_address(Address::new(
            Uri::from_str(&format!("sip:{proxy};lr")).unwrap(),
        ))));
    let route = TransportRoute::new(proxy)
        .with_transport_type(TransportType::Tcp)
        .with_authority(TransportAuthority::ip(proxy.ip()));
    let transaction = uac
        .manager
        .create_client_transaction_on_route(invite, route)
        .await
        .expect("INVITE transaction");
    uac.manager
        .send_request(&transaction)
        .await
        .expect("INVITE send");
}

fn spawn_ack_sender(uac: Uac, resolver: Option<Arc<dyn Resolver>>) -> Arc<TransactionManager> {
    let Uac {
        manager,
        mut events,
        ..
    } = uac;
    let ack_manager = manager.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let TransactionEvent::SuccessResponse {
                transaction_id,
                response,
                ..
            } = event
            {
                ack_manager
                    .send_ack_for_2xx_with_resolver(&transaction_id, &response, resolver.clone())
                    .await
                    .expect("ACK send");
            }
        }
    });
    manager
}

async fn read_for(stream: &mut TcpStream, window: Duration) -> String {
    let mut text = String::new();
    let mut buf = vec![0u8; 65536];
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Ok(read)) = tokio::time::timeout_at(deadline, stream.read(&mut buf)).await {
        if read == 0 {
            break;
        }
        text.push_str(&String::from_utf8_lossy(&buf[..read]));
    }
    text
}

async fn answer(stream: &mut TcpStream, invite_wire: &str, record_route: &str) {
    let Message::Request(invite) =
        rvoip_sip_core::parse_message(invite_wire.as_bytes()).expect("INVITE parses")
    else {
        panic!("proxy expected a request");
    };
    let mut ok = SimpleResponseBuilder::response_from_request(&invite, StatusCode::Ok, Some("OK"))
        .to("Bob", "sip:bob@example.test", Some("bob-tag"))
        .contact("sip:bob@127.0.0.1:9", None)
        .build();
    ok.headers.push(TypedHeader::RecordRoute(
        RecordRoute::from_str(record_route).expect("Record-Route"),
    ));
    stream
        .write_all(&Message::Response(ok).to_bytes())
        .await
        .expect("2xx write");
}

fn ack_via(wire: &str) -> Option<String> {
    wire.lines()
        .skip_while(|line| !line.starts_with("ACK "))
        .find(|line| line.to_ascii_lowercase().starts_with("via:"))
        .map(str::to_owned)
}

struct Observed {
    tcp: String,
    udp: String,
    new_connection: bool,
}

async fn run(
    record_route: &str,
    resolver_for: impl FnOnce(SocketAddr) -> Option<Arc<dyn Resolver>>,
    call_id: &str,
) -> Observed {
    let proxy = proxy().await;
    let resolver = resolver_for(proxy.addr);
    let uac = uac().await;
    send_invite(&uac, proxy.addr, call_id).await;
    let (mut stream, _) = proxy.tcp.accept().await.expect("INVITE connection");
    let invite_wire = read_for(&mut stream, Duration::from_millis(150)).await;
    let manager = spawn_ack_sender(uac, resolver);

    let record_route = record_route.replace("PROXY", &proxy.addr.to_string());
    answer(&mut stream, &invite_wire, &record_route).await;

    let mut buf = vec![0u8; 65536];
    let tcp_read = read_for(&mut stream, Duration::from_millis(500));
    let udp_read = tokio::time::timeout(Duration::from_millis(500), proxy.udp.recv_from(&mut buf));
    let accept = tokio::time::timeout(Duration::from_millis(500), proxy.tcp.accept());
    let (tcp, udp_result, accepted) = tokio::join!(tcp_read, udp_read, accept);
    let udp = match udp_result {
        Ok(Ok((read, _))) => String::from_utf8_lossy(&buf[..read]).into_owned(),
        _ => String::new(),
    };

    manager.shutdown().await;
    Observed {
        tcp,
        udp,
        new_connection: accepted.is_ok(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ack_over_udp_declares_udp_when_record_route_has_no_transport() {
    let observed = run("<sip:PROXY;lr>", |_| None, "ack-rr-ip").await;

    assert!(!observed.tcp.contains("ACK sip:"), "ACK must not use TCP");
    assert!(observed.udp.starts_with("ACK sip:"), "ACK must use UDP");
    let via = ack_via(&observed.udp).expect("ACK Via");
    assert!(via.contains("SIP/2.0/UDP"), "unexpected ACK {via}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ack_reuses_tcp_connection_when_record_route_requests_tcp() {
    let observed = run("<sip:PROXY;transport=tcp;lr>", |_| None, "ack-rr-tcp").await;

    assert!(observed.udp.is_empty(), "ACK must not use UDP");
    assert_eq!(observed.tcp.matches("ACK sip:").count(), 1);
    let via = ack_via(&observed.tcp).expect("ACK Via");
    assert!(via.contains("SIP/2.0/TCP"), "unexpected ACK {via}");
    assert!(
        !observed.new_connection,
        "ACK must reuse the INVITE connection"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ack_resolves_hostname_record_route_onto_the_invite_connection() {
    let observed = run(
        &format!("<sip:{PROXY_HOST};lr>"),
        |proxy| {
            let resolver: Arc<dyn Resolver> = Arc::new(FixedResolver(ResolvedTarget {
                addr: proxy,
                transport: TransportType::Tcp,
                authority: Some(TransportAuthority::ip(proxy.ip())),
                expires: None,
            }));
            Some(resolver)
        },
        "ack-rr-host",
    )
    .await;

    assert!(observed.udp.is_empty(), "ACK must not use UDP");
    assert_eq!(observed.tcp.matches("ACK sip:").count(), 1);
    let via = ack_via(&observed.tcp).expect("ACK Via");
    assert!(via.contains("SIP/2.0/TCP"), "unexpected ACK {via}");
    assert!(
        !observed.new_connection,
        "ACK must reuse the INVITE connection"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn slow_proxy_receives_a_single_invite_write_over_tcp() {
    let proxy = proxy().await;
    let uac = uac().await;
    send_invite(&uac, proxy.addr, "invite-no-tcp-retransmit").await;
    let (mut stream, _) = proxy.tcp.accept().await.expect("INVITE connection");

    // Timer A would retransmit at T1 (500 ms) and 1.5 s on an unreliable route.
    let received = read_for(&mut stream, Duration::from_millis(1700)).await;
    assert_eq!(received.matches("INVITE sip:").count(), 1, "{received}");

    uac.manager.shutdown().await;
}
