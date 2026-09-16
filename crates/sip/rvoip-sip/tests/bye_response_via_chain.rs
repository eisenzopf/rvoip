//! A BYE forwarded by a proxy carries the proxy Via above the client Via. The
//! 200 OK from a real rvoip peer must echo both, in order (RFC 3261
//! §8.2.6.2), so the proxy can strip its own and still reach the client.

use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::Config;
use tokio::net::{TcpListener, UdpSocket};
use tokio::time::{sleep, timeout};

const SDP: &str = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 35299 RTP/AVP 0\r\n";
const CLIENT_BRANCH: &str = "z9hG4bK-bye-client-leg";
const PROXY_BRANCH: &str = "z9hG4bK-bye-proxy-leg";

struct AcceptAll;

#[async_trait::async_trait]
impl CallHandler for AcceptAll {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
        CallHandlerDecision::Accept
    }
}

async fn reserve_port() -> u16 {
    loop {
        let tcp = TcpListener::bind("127.0.0.1:0").await.expect("TCP bind");
        let addr = tcp.local_addr().expect("TCP address");
        if UdpSocket::bind(addr).await.is_ok() {
            return addr.port();
        }
    }
}

async fn recv_text(sock: &UdpSocket, starts_with: &str) -> String {
    let mut buf = vec![0u8; 65536];
    timeout(Duration::from_secs(8), async {
        loop {
            let (n, _) = sock.recv_from(&mut buf).await.expect("recv");
            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
            if text.starts_with(starts_with) {
                return text;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("expected a message starting with {starts_with}"))
}

fn header_value<'a>(message: &'a str, name: &str) -> Option<&'a str> {
    message.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bye_200_keeps_the_proxy_and_client_vias_in_order() {
    let proxy_port = reserve_port().await;
    let bob_port = reserve_port().await;
    let proxy = UdpSocket::bind(format!("127.0.0.1:{proxy_port}"))
        .await
        .expect("proxy bind");

    let config = Config::local("bob", bob_port).with_auto_180_ringing(false);
    let bob = CallbackPeer::new(AcceptAll, config)
        .await
        .expect("bob CallbackPeer");
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    sleep(Duration::from_millis(200)).await;
    let bob_addr = format!("127.0.0.1:{bob_port}");

    let invite = format!(
        "INVITE sip:bob@{bob_addr} SIP/2.0\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:{proxy_port};branch=z9hG4bK-via-chain-invite\r\n\
         Max-Forwards: 70\r\n\
         Record-Route: <sip:127.0.0.1:{proxy_port};lr>\r\n\
         From: <sip:carol@127.0.0.1>;tag=carol-tag\r\n\
         To: <sip:bob@{bob_addr}>\r\n\
         Call-ID: via-chain@127.0.0.1\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: <sip:carol@127.0.0.1:{proxy_port}>\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: {}\r\n\r\n{SDP}",
        SDP.len()
    );
    proxy
        .send_to(invite.as_bytes(), &bob_addr)
        .await
        .expect("INVITE");
    let ok = recv_text(&proxy, "SIP/2.0 200").await;
    let to = header_value(&ok, "To").expect("2xx To").to_string();

    let ack = format!(
        "ACK sip:bob@{bob_addr} SIP/2.0\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:{proxy_port};branch=z9hG4bK-via-chain-ack\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:carol@127.0.0.1>;tag=carol-tag\r\n\
         To: {to}\r\n\
         Call-ID: via-chain@127.0.0.1\r\n\
         CSeq: 1 ACK\r\n\
         Content-Length: 0\r\n\r\n"
    );
    proxy.send_to(ack.as_bytes(), &bob_addr).await.expect("ACK");
    sleep(Duration::from_millis(300)).await;

    let bye = format!(
        "BYE sip:bob@{bob_addr} SIP/2.0\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:{proxy_port};branch={PROXY_BRANCH}\r\n\
         Via: SIP/2.0/UDP 192.0.2.10:5090;branch={CLIENT_BRANCH};rport\r\n\
         Max-Forwards: 69\r\n\
         From: <sip:carol@127.0.0.1>;tag=carol-tag\r\n\
         To: {to}\r\n\
         Call-ID: via-chain@127.0.0.1\r\n\
         CSeq: 2 BYE\r\n\
         Content-Length: 0\r\n\r\n"
    );
    proxy.send_to(bye.as_bytes(), &bob_addr).await.expect("BYE");
    let response = recv_text(&proxy, "SIP/2.0 200").await;

    let vias: Vec<&str> = response
        .lines()
        .filter(|line| line.to_ascii_lowercase().starts_with("via:"))
        .collect();
    assert_eq!(vias.len(), 2, "{response}");
    assert!(vias[0].contains(PROXY_BRANCH), "{response}");
    assert!(vias[1].contains(CLIENT_BRANCH), "{response}");
    assert!(vias[1].contains("192.0.2.10:5090"), "{response}");
    assert!(
        header_value(&response, "CSeq").is_some_and(|cseq| cseq.ends_with("BYE")),
        "{response}"
    );

    bob_shutdown.shutdown();
    let _ = timeout(Duration::from_secs(2), bob_task).await;
}
