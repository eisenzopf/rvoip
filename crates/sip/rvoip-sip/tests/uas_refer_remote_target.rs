//! Regression for in-dialog requests sent by a UAS whose caller uses an
//! address-of-record in `From` and a different, reachable `Contact`.
//!
//! RFC 3261 §12.2.1.1 sends in-dialog requests to the remote target learned
//! from the peer Contact. Loopback tests usually give the caller the same
//! `sip:user@127.0.0.1:port` URI in both headers, which hides a Request-URI
//! built from the remote URI instead. Here the `From` host does not resolve,
//! so such a request never reaches the wire.

use std::time::Duration;

use rvoip_sip::api::unified::Config;
use rvoip_sip::{CallState, StreamPeer};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::{Message, Method, StatusCode};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use serial_test::serial;
use tokio::net::UdpSocket;
use tokio::sync::oneshot;

const UAS_PORT: u16 = 17_661;
const CALLER_AOR: &str = "sip:caller@carrier.invalid";

fn uas_config() -> Config {
    let mut config = Config::local("refer-uas", UAS_PORT);
    config.media_port_start = 28_200;
    config.media_port_end = 28_300;
    config
}

fn header(message: &str, name: &str) -> String {
    let prefix = format!("{}:", name.to_ascii_lowercase());
    message
        .split("\r\n")
        .find(|line| line.to_ascii_lowercase().starts_with(&prefix))
        .map(|line| line[prefix.len()..].trim().to_string())
        .unwrap_or_else(|| panic!("missing {name} header"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn uas_refer_targets_caller_contact_not_from_aor() {
    let mut uas = StreamPeer::with_config(uas_config())
        .await
        .expect("UAS peer");
    let coordinator = uas.coordinator().clone();

    let caller = UdpSocket::bind("127.0.0.1:0").await.expect("caller bind");
    let caller_addr = caller.local_addr().expect("caller address");
    let caller_contact = format!("sip:caller@{caller_addr}");
    let sdp = format!(
        "v=0\r\no=caller 1 1 IN IP4 127.0.0.1\r\ns=refer-target\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n",
        caller_addr.port() + 2
    );
    let invite = format!(
        "INVITE sip:refer-uas@127.0.0.1:{UAS_PORT} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {caller_addr};branch=z9hG4bK-refer-target-invite;rport\r\n\
         Max-Forwards: 70\r\n\
         From: <{CALLER_AOR}>;tag=refer-target-caller\r\n\
         To: <sip:refer-uas@127.0.0.1:{UAS_PORT}>\r\n\
         Call-ID: refer-target@carrier.invalid\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: <{caller_contact}>\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: {}\r\n\r\n{sdp}",
        sdp.len()
    );

    let (refer_tx, refer_rx) = oneshot::channel::<String>();
    let caller_task = tokio::spawn(async move {
        let uas_addr = format!("127.0.0.1:{UAS_PORT}");
        caller
            .send_to(invite.as_bytes(), &uas_addr)
            .await
            .expect("send INVITE");
        let mut refer_tx = Some(refer_tx);
        let mut packet = vec![0u8; 65_535];
        loop {
            let (length, peer) = caller.recv_from(&mut packet).await.expect("caller receive");
            let text = String::from_utf8_lossy(&packet[..length]).into_owned();
            match parse_message(&packet[..length]).expect("parse UAS message") {
                Message::Response(response)
                    if response.status_code() == 200
                        && response
                            .cseq()
                            .is_some_and(|cseq| cseq.method == Method::Invite) =>
                {
                    let ack = format!(
                        "ACK sip:refer-uas@127.0.0.1:{UAS_PORT} SIP/2.0\r\n\
                         Via: SIP/2.0/UDP {caller_addr};branch=z9hG4bK-refer-target-ack;rport\r\n\
                         Max-Forwards: 70\r\n\
                         From: {}\r\n\
                         To: {}\r\n\
                         Call-ID: refer-target@carrier.invalid\r\n\
                         CSeq: 1 ACK\r\n\
                         Content-Length: 0\r\n\r\n",
                        header(&text, "From"),
                        header(&text, "To"),
                    );
                    caller
                        .send_to(ack.as_bytes(), peer)
                        .await
                        .expect("send ACK");
                }
                Message::Response(_) => {}
                Message::Request(request) => {
                    let method = request.method();
                    let response = match method {
                        Method::Refer => create_response(&request, StatusCode::Accepted),
                        _ => create_response(&request, StatusCode::Ok),
                    };
                    caller
                        .send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("answer UAS request");
                    if method == Method::Refer {
                        if let Some(sender) = refer_tx.take() {
                            let _ = sender.send(text);
                        }
                    }
                    if method == Method::Bye {
                        return;
                    }
                }
            }
        }
    });

    let incoming = tokio::time::timeout(Duration::from_secs(5), uas.wait_for_incoming())
        .await
        .expect("INVITE never reached the UAS")
        .expect("incoming call");
    let handle = incoming.accept().await.expect("accept INVITE");
    let call_id = handle.id().clone();

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(coordinator.get_state(&call_id).await, Ok(CallState::Active)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("UAS never became active after ACK");

    tokio::time::timeout(
        Duration::from_secs(5),
        // Transfer targets are often held in name-addr form, as they appear
        // in the Refer-To header itself.
        coordinator
            .refer(&call_id, "\"Carol\" <sip:carol@127.0.0.1>")
            .send(),
    )
    .await
    .expect("REFER dispatch timed out")
    .expect("UAS REFER must dispatch when the caller From host does not resolve");

    let refer = tokio::time::timeout(Duration::from_secs(5), refer_rx)
        .await
        .expect("REFER never reached the caller Contact")
        .expect("caller task ended before REFER");
    let request_line = refer.split("\r\n").next().expect("REFER request line");
    assert_eq!(request_line, format!("REFER {caller_contact} SIP/2.0"));
    assert!(
        header(&refer, "To").contains(CALLER_AOR),
        "To must keep the caller address-of-record"
    );
    assert_eq!(header(&refer, "Refer-To"), "<sip:carol@127.0.0.1>");

    coordinator
        .hangup(&call_id)
        .await
        .expect("UAS hangup after REFER");
    tokio::time::timeout(Duration::from_secs(5), caller_task)
        .await
        .expect("caller never received BYE")
        .expect("caller task");
    let _ = uas.shutdown().await;
}
