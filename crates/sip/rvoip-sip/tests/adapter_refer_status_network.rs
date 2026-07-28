//! Real localhost REFER/NOTIFY coverage for transport-neutral transfer status.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rvoip_core::adapter::{
    AdapterEvent, ConnectionAdapter, EndReason, OriginateRequest, TransferStatus, TransferTarget,
};
use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

const UAS_TAG: &str = "refer-status-uas";

#[derive(Debug)]
struct EstablishedDialog {
    peer: std::net::SocketAddr,
    call_id: String,
    local_from: String,
    remote_to: String,
}

async fn send_notify(
    socket: &UdpSocket,
    uas_addr: std::net::SocketAddr,
    dialog: &EstablishedDialog,
    cseq: u32,
    status_code: u16,
    reason: &str,
    terminal: bool,
) {
    let body = format!("SIP/2.0 {status_code} {reason}\r\n");
    let subscription_state = if terminal {
        "terminated;reason=noresource"
    } else {
        "active;expires=60"
    };
    let headers = format!(
        "NOTIFY sip:refer-uac@{} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {uas_addr};branch=z9hG4bK-refer-notify-{cseq};rport\r\n\
         From: {}\r\n\
         To: {}\r\n\
         Call-ID: {}\r\n\
         CSeq: {cseq} NOTIFY\r\n\
         Max-Forwards: 70\r\n\
         Contact: <sip:refer@{uas_addr}>\r\n\
         Event: refer\r\n\
         Subscription-State: {subscription_state}\r\n\
         Content-Type: message/sipfrag\r\n\
         Content-Length: {}\r\n\r\n",
        dialog.peer,
        dialog.remote_to,
        dialog.local_from,
        dialog.call_id,
        body.len(),
    );
    let mut wire = headers.into_bytes();
    wire.extend_from_slice(body.as_bytes());
    socket
        .send_to(&wire, dialog.peer)
        .await
        .expect("send NOTIFY");
}

async fn wait_for_transfer_status(
    events: &mut mpsc::Receiver<AdapterEvent>,
    connection_id: &rvoip_core::ids::ConnectionId,
) -> TransferStatus {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::TransferStatus {
                    connection_id: id,
                    status,
                    ..
                }) if &id == connection_id => return status,
                Some(_) => {}
                None => panic!("adapter event stream closed before transfer status"),
            }
        }
    })
    .await
    .expect("transfer status deadline")
}

async fn run_refer_notify_case(final_status: u16, final_reason: &str) {
    let uas = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("UAS bind"));
    let uas_addr = uas.local_addr().expect("UAS address");
    let media_sink = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("media sink bind");
    let media_port = media_sink.local_addr().expect("media address").port();
    let (established_tx, established_rx) = oneshot::channel();
    let (refer_tx, mut refer_rx) = mpsc::unbounded_channel();
    let (notify_response_tx, mut notify_response_rx) = mpsc::unbounded_channel();
    let task_socket = Arc::clone(&uas);
    let uas_task = tokio::spawn(async move {
        let mut packet = vec![0u8; 65_536];
        let mut established_tx = Some(established_tx);
        loop {
            let (bytes, peer) = task_socket
                .recv_from(&mut packet)
                .await
                .expect("UAS receive");
            match parse_message(&packet[..bytes]).expect("parse SIP message") {
                Message::Request(request) => match request.method() {
                    Method::Invite => {
                        let mut response = create_response(&request, StatusCode::Ok);
                        if let Some(TypedHeader::To(to)) = response
                            .headers
                            .iter_mut()
                            .find(|header| matches!(header, TypedHeader::To(_)))
                        {
                            to.set_tag(UAS_TAG);
                        }
                        response.headers.push(TypedHeader::Contact(
                            Contact::from_str(&format!("<sip:refer@{uas_addr}>"))
                                .expect("UAS Contact"),
                        ));
                        response.headers.push(TypedHeader::ContentType(
                            rvoip_sip_core::types::ContentType::sdp(),
                        ));
                        response.body = Bytes::from(format!(
                            "v=0\r\no=refer 1 1 IN IP4 127.0.0.1\r\ns=refer-status\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n"
                        ));
                        response
                            .headers
                            .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
                        response.headers.push(TypedHeader::ContentLength(
                            rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
                        ));
                        task_socket
                            .send_to(&Message::Response(response).to_bytes(), peer)
                            .await
                            .expect("send INVITE response");
                    }
                    Method::Ack => {
                        if let Some(sender) = established_tx.take() {
                            sender
                                .send(EstablishedDialog {
                                    peer,
                                    call_id: request
                                        .call_id()
                                        .map(|value| value.value())
                                        .expect("ACK Call-ID"),
                                    local_from: request
                                        .raw_header_value(&HeaderName::From)
                                        .expect("ACK From"),
                                    remote_to: request
                                        .raw_header_value(&HeaderName::To)
                                        .expect("ACK To"),
                                })
                                .expect("established receiver");
                        }
                    }
                    Method::Refer => {
                        refer_tx.send(request.clone()).expect("REFER receiver");
                        let response = create_response(&request, StatusCode::Accepted);
                        task_socket
                            .send_to(&Message::Response(response).to_bytes(), peer)
                            .await
                            .expect("send REFER response");
                    }
                    Method::Bye | Method::Cancel => {
                        let response = create_response(&request, StatusCode::Ok);
                        task_socket
                            .send_to(&Message::Response(response).to_bytes(), peer)
                            .await
                            .expect("send teardown response");
                    }
                    _ => {}
                },
                Message::Response(response) => {
                    if response
                        .cseq()
                        .is_some_and(|cseq| cseq.method == Method::Notify)
                    {
                        notify_response_tx
                            .send(response.status().as_u16())
                            .expect("NOTIFY response receiver");
                    }
                }
            }
        }
    });

    let coordinator = UnifiedCoordinator::new(SipConfig::local("refer-uac", 0))
        .await
        .expect("coordinator");
    let adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("adapter");
    let mut events = ConnectionAdapter::subscribe_events(adapter.as_ref());
    let prepared = ConnectionAdapter::originate(
        adapter.as_ref(),
        OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            format!("sip:refer@{uas_addr}"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip),
    )
    .await
    .expect("prepare route");
    let connection_id = prepared.connection.id.clone();
    ConnectionAdapter::activate_outbound_with_receipt(adapter.as_ref(), connection_id.clone())
        .await
        .expect("activate route");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::Connected { connection_id: id }) if id == connection_id => break,
                Some(_) => {}
                None => panic!("event stream closed before Connected"),
            }
        }
    })
    .await
    .expect("Connected deadline");
    let established = tokio::time::timeout(Duration::from_secs(5), established_rx)
        .await
        .expect("ACK deadline")
        .expect("ACK dialog");

    let target = "sip:transfer-target@example.test";
    ConnectionAdapter::transfer(
        adapter.as_ref(),
        connection_id.clone(),
        TransferTarget::Uri(target.into()),
    )
    .await
    .expect("submit REFER");
    let refer = tokio::time::timeout(Duration::from_secs(5), refer_rx.recv())
        .await
        .expect("REFER deadline")
        .expect("REFER request");
    assert_eq!(
        refer.call_id().map(|value| value.value()).as_deref(),
        Some(established.call_id.as_str())
    );
    assert!(refer
        .raw_header_value(&HeaderName::ReferTo)
        .is_some_and(|value| value.contains(target)));

    send_notify(&uas, uas_addr, &established, 10, 180, "Ringing", false).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), notify_response_rx.recv())
            .await
            .expect("progress NOTIFY response deadline")
            .expect("progress NOTIFY response"),
        200
    );
    assert!(matches!(
        wait_for_transfer_status(&mut events, &connection_id).await,
        TransferStatus::Progress {
            status_code: 180,
            ref reason,
        } if reason == "Ringing"
    ));

    send_notify(
        &uas,
        uas_addr,
        &established,
        11,
        final_status,
        final_reason,
        true,
    )
    .await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), notify_response_rx.recv())
            .await
            .expect("final NOTIFY response deadline")
            .expect("final NOTIFY response"),
        200
    );
    let final_update = wait_for_transfer_status(&mut events, &connection_id).await;
    if (200..300).contains(&final_status) {
        assert!(matches!(
            final_update,
            TransferStatus::Completed {
                status_code,
                ref reason,
            } if status_code == final_status && reason == final_reason
        ));
    } else {
        assert!(matches!(
            final_update,
            TransferStatus::Failed {
                status_code,
                ref reason,
            } if status_code == final_status && reason == final_reason
        ));
    }

    ConnectionAdapter::end(adapter.as_ref(), connection_id, EndReason::Normal)
        .await
        .expect("end route");
    adapter.drain().await.expect("adapter drain");
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("coordinator shutdown");
    uas_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refer_notify_progress_and_final_outcomes_are_typed_on_the_real_wire() {
    run_refer_notify_case(200, "OK").await;
    run_refer_notify_case(503, "Service Unavailable").await;
}
