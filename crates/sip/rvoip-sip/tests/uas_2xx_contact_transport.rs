//! INVITE over TCP to a real UAS, whose 2xx Contact must advertise TCP so the
//! UAC sends the ACK back over TCP (RFC 3261 §12.1.1, RFC 3263 §4.1).
//!
//! The UAS advertises a separate observer address, so the UAC has to route
//! the ACK from the Contact instead of reusing the INVITE connection. The
//! observer listens on TCP and UDP at the same port and records where the
//! ACK arrives.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::Config;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{HeaderName, Method};
use rvoip_sip_dialog::transaction::transport::MultiplexedTransport;
use rvoip_sip_dialog::transaction::{TransactionEvent, TransactionManager};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{
    TcpTransport, Transport, TransportAuthority, TransportEvent, TransportRoute, UdpTransport,
};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;

struct AcceptAll;

#[async_trait::async_trait]
impl CallHandler for AcceptAll {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
        CallHandlerDecision::Accept
    }
}

async fn free_udp_and_tcp_port() -> (TcpListener, UdpSocket) {
    loop {
        let tcp = TcpListener::bind("127.0.0.1:0").await.expect("TCP bind");
        let addr = tcp.local_addr().expect("TCP address");
        if let Ok(udp) = UdpSocket::bind(addr).await {
            return (tcp, udp);
        }
    }
}

async fn reserve_port() -> u16 {
    let (tcp, udp) = free_udp_and_tcp_port().await;
    let port = tcp.local_addr().expect("reserved address").port();
    drop((tcp, udp));
    port
}

async fn uac() -> (
    Arc<TransactionManager>,
    mpsc::Receiver<TransactionEvent>,
    SocketAddr,
) {
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
    (Arc::new(manager), events, local)
}

#[tokio::test(flavor = "multi_thread")]
async fn tcp_invite_gets_tcp_contact_and_tcp_ack() {
    let (observer_tcp, observer_udp) = free_udp_and_tcp_port().await;
    let observer = observer_tcp.local_addr().expect("observer address");

    let uas_port = reserve_port().await;
    let mut uas_config = Config::local("server", uas_port).with_auto_180_ringing(false);
    uas_config.sip_advertised_addr = Some(observer);
    let uas = CallbackPeer::new(AcceptAll, uas_config)
        .await
        .expect("UAS CallbackPeer::new");
    let uas_shutdown = uas.shutdown_handle();
    let uas_task = tokio::spawn(async move {
        let _ = uas.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let uas_addr: SocketAddr = format!("127.0.0.1:{uas_port}").parse().unwrap();

    let (manager, mut events, local) = uac().await;
    let sdp = format!(
        "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
         m=audio {} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n",
        reserve_port().await
    );
    let invite = SimpleRequestBuilder::new(Method::Invite, &format!("sip:server@{uas_addr}"))
        .unwrap()
        .from("Alice", "sip:alice@example.test", Some("alice-tag"))
        .to("Server", &format!("sip:server@{uas_addr}"), None)
        .contact(&format!("sip:alice@{local};transport=tcp"), None)
        .call_id("uas-contact-transport")
        .cseq(1)
        .via(&local.to_string(), "TCP", Some("z9hG4bK.uas-contact"))
        .max_forwards(70)
        .content_type("application/sdp")
        .body(sdp.into_bytes())
        .build();
    let route = TransportRoute::new(uas_addr)
        .with_transport_type(TransportType::Tcp)
        .with_authority(TransportAuthority::ip(uas_addr.ip()));
    let transaction = manager
        .create_client_transaction_on_route(invite, route)
        .await
        .expect("INVITE transaction");
    manager
        .send_request(&transaction)
        .await
        .expect("INVITE send");

    let contact = tokio::time::timeout(Duration::from_secs(8), async {
        while let Some(event) = events.recv().await {
            if let TransactionEvent::SuccessResponse {
                transaction_id,
                response,
                ..
            } = event
            {
                let contact = match response.header(&HeaderName::Contact) {
                    Some(TypedHeader::Contact(contact)) => contact
                        .addresses()
                        .next()
                        .map(|address| address.uri.to_string()),
                    _ => None,
                };
                manager
                    .send_ack_for_2xx(&transaction_id, &response)
                    .await
                    .expect("ACK send");
                return contact;
            }
        }
        None
    })
    .await
    .expect("2xx from the UAS")
    .expect("2xx Contact");
    assert_eq!(contact, format!("sip:server@{observer};transport=tcp"));

    let mut udp_buf = vec![0u8; 65536];
    let received = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::select! {
            accepted = observer_tcp.accept() => {
                let (mut stream, _) = accepted.expect("ACK connection");
                let mut buf = vec![0u8; 65536];
                let mut wire = String::new();
                while !wire.contains("\r\n\r\n") {
                    let read = stream.read(&mut buf).await.expect("ACK read");
                    if read == 0 {
                        break;
                    }
                    wire.push_str(&String::from_utf8_lossy(&buf[..read]));
                }
                ("TCP", wire)
            }
            datagram = observer_udp.recv_from(&mut udp_buf) => {
                let (read, _) = datagram.expect("ACK datagram");
                ("UDP", String::from_utf8_lossy(&udp_buf[..read]).into_owned())
            }
        }
    })
    .await
    .expect("ACK must reach the Contact address");

    assert_eq!(received.0, "TCP", "ACK arrived over {}", received.0);
    assert!(received.1.starts_with("ACK sip:server@"), "{}", received.1);
    assert!(received.1.contains("SIP/2.0/TCP"), "{}", received.1);

    manager.shutdown().await;
    uas_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), uas_task).await;
}
