//! The TU may be waiting for a response while its bounded event queue is full.
//! Successful wire completion and the exact state transition must not depend on
//! that same consumer draining its queue. Events must still arrive in order.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::server::{
    ServerInviteTransaction, ServerNonInviteTransaction, ServerTransaction,
};
use rvoip_sip_dialog::transaction::timer::TimerSettings;
use rvoip_sip_dialog::transaction::{TransactionEvent, TransactionKey, TransactionState};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{Transport, TransportRoute};
use tokio::sync::mpsc;

#[derive(Debug, Default)]
struct RecordingTransport {
    messages: Mutex<Vec<Message>>,
}

#[async_trait::async_trait]
impl Transport for RecordingTransport {
    fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
        Ok("127.0.0.1:5090".parse().unwrap())
    }

    async fn send_message(
        &self,
        message: Message,
        _destination: SocketAddr,
    ) -> std::result::Result<(), rvoip_sip_transport::Error> {
        self.messages.lock().unwrap().push(message);
        Ok(())
    }

    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

async fn response_completes_with_full_tu_queue(method: Method, status: StatusCode, reliable: bool) {
    let invite = method == Method::Invite;
    let identity = uuid::Uuid::new_v4().to_string();
    let branch = format!("z9hG4bK.{identity}");
    let request = SimpleRequestBuilder::new(method, "sip:bob@example.com")
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some("from-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id(&identity)
        .cseq(1)
        .via(
            "127.0.0.1:5070",
            if reliable { "TCP" } else { "UDP" },
            Some(&branch),
        )
        .build();
    let key = TransactionKey::from_request(&request).unwrap();
    let (events_tx, mut events_rx) = mpsc::channel(1);
    events_tx
        .try_send(TransactionEvent::TimerTriggered {
            transaction_id: key.clone(),
            timer: "queue-sentinel".into(),
        })
        .unwrap();
    let transport = Arc::new(RecordingTransport::default());
    let settings = TimerSettings {
        t1: Duration::from_secs(10),
        t2: Duration::from_secs(10),
        wait_time_h: Duration::from_secs(30),
        wait_time_j: Duration::from_secs(30),
        ..Default::default()
    };
    let response = SimpleResponseBuilder::response_from_request(&request, status, None).build();
    let route =
        TransportRoute::new("127.0.0.1:5070".parse().unwrap()).with_transport_type(if reliable {
            TransportType::Tcp
        } else {
            TransportType::Udp
        });
    let transaction: Arc<dyn ServerTransaction> = if invite {
        Arc::new(
            ServerInviteTransaction::new_with_response_route_and_command_channel_capacity(
                key.clone(),
                request,
                route,
                transport.clone(),
                events_tx,
                Some(settings),
                16,
            )
            .unwrap(),
        )
    } else {
        Arc::new(
            ServerNonInviteTransaction::new_with_response_route_and_command_channel_capacity(
                key.clone(),
                request,
                route,
                transport.clone(),
                events_tx,
                Some(settings),
                16,
            )
            .unwrap(),
        )
    };

    // Deliberately do not receive an event until response completion. This is
    // the causal cycle a dialog handler encounters under queue backpressure.
    tokio::time::timeout(Duration::from_secs(1), transaction.send_response(response))
        .await
        .expect("written response must complete before the TU queue is drained")
        .expect("response write succeeded");
    let expected_state = if invite && status.is_success() {
        TransactionState::Terminated // Public projection of RFC 6026 Accepted.
    } else {
        TransactionState::Completed
    };
    assert_eq!(transaction.state(), expected_state);
    if invite && status.is_success() {
        assert!(
            !transaction.is_protocol_terminated(),
            "Accepted must retain RFC 6026 ownership"
        );
    }
    {
        let messages = transport.messages.lock().unwrap();
        assert_eq!(messages.len(), 1, "exactly one transport write");
        assert!(matches!(&messages[0], Message::Response(response) if response.status() == status));
    }

    assert!(
        matches!(events_rx.recv().await, Some(TransactionEvent::TimerTriggered { timer, .. }) if timer == "queue-sentinel")
    );
    let transition = tokio::time::timeout(Duration::from_secs(1), events_rx.recv())
        .await
        .unwrap()
        .expect("retained state event");
    assert!(
        matches!(transition, TransactionEvent::StateChanged { transaction_id, new_state, .. }
        if transaction_id == key && new_state == expected_state)
    );
    if reliable && !invite {
        // Timer J is zero on reliable transports. The queued transition must
        // still be observed after Completed, with no lost terminal event.
        let mut states = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(event) = events_rx.recv().await {
                match event {
                    TransactionEvent::StateChanged { new_state, .. } => states.push(new_state),
                    TransactionEvent::TransactionTerminated { transaction_id } => {
                        assert_eq!(transaction_id, key);
                        break;
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("ordered reliable termination");
        assert_eq!(states, vec![TransactionState::Terminated]);
        assert!(transaction.is_protocol_terminated());
    }
}

#[tokio::test]
async fn invite_200_completes_under_tu_backpressure() {
    response_completes_with_full_tu_queue(Method::Invite, StatusCode::Ok, false).await;
}

#[tokio::test]
async fn invite_503_completes_under_tu_backpressure() {
    response_completes_with_full_tu_queue(Method::Invite, StatusCode::ServiceUnavailable, false)
        .await;
}

#[tokio::test]
async fn non_invite_200_completes_under_tu_backpressure() {
    response_completes_with_full_tu_queue(Method::Bye, StatusCode::Ok, false).await;
}

#[tokio::test]
async fn reliable_non_invite_retains_ordered_terminal_events_under_tu_backpressure() {
    response_completes_with_full_tu_queue(Method::Bye, StatusCode::Ok, true).await;
}
