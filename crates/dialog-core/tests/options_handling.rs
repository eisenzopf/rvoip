use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_dialog_core::events::DialogEventHub;
use rvoip_dialog_core::transaction::{TransactionManager, TransactionState};
use rvoip_dialog_core::{DialogManager, DialogManagerConfig};
use rvoip_infra_common::events::{EventCoordinatorConfig, GlobalEventCoordinator};
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::{ContentLength, HeaderName, TypedHeader};
use rvoip_sip_core::{Message, Method, Request, Response};
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone)]
struct MockTransport {
    local_addr: SocketAddr,
    sent_messages: Arc<Mutex<Vec<(Message, SocketAddr)>>>,
}

impl MockTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            sent_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn sent_messages(&self) -> Vec<(Message, SocketAddr)> {
        self.sent_messages.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for MockTransport {
    async fn send_message(
        &self,
        message: Message,
        destination: SocketAddr,
    ) -> Result<(), rvoip_sip_transport::Error> {
        self.sent_messages.lock().await.push((message, destination));
        Ok(())
    }

    fn local_addr(&self) -> Result<SocketAddr, rvoip_sip_transport::Error> {
        Ok(self.local_addr)
    }

    async fn close(&self) -> Result<(), rvoip_sip_transport::Error> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

fn options_request() -> Request {
    SimpleRequestBuilder::new(Method::Options, "sip:1001@example.com")
        .unwrap()
        .from("Asterisk", "sip:asterisk@example.com", Some("ast-tag"))
        .to("Endpoint", "sip:1001@example.com", None)
        .call_id("options-call-id")
        .cseq(42)
        .via("192.0.2.10:5060", "UDP", Some("z9hG4bK-options-test"))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

async fn manager_with_options_config(
    auto_options: bool,
    attach_event_hub: bool,
) -> Result<
    (
        Arc<DialogManager>,
        Arc<TransactionManager>,
        Arc<MockTransport>,
    ),
    Box<dyn std::error::Error>,
> {
    let local_addr = SocketAddr::from_str("127.0.0.1:5060")?;
    let transport = Arc::new(MockTransport::new(local_addr));
    let (_transport_tx, transport_rx) = mpsc::channel(8);
    let (transaction_manager, _event_rx) =
        TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;
    let transaction_manager = Arc::new(transaction_manager);

    let mut manager = DialogManager::new(transaction_manager.clone(), local_addr).await?;
    let config = DialogManagerConfig::hybrid(local_addr).with_from_uri("sip:1001@example.com");
    let config = if auto_options {
        config.with_auto_options()
    } else {
        config
    };
    manager.set_config(config.build());

    let manager = Arc::new(manager);
    if attach_event_hub {
        let coordinator =
            Arc::new(GlobalEventCoordinator::new(EventCoordinatorConfig::default()).await?);
        let hub = DialogEventHub::new(coordinator, manager.clone()).await?;
        manager.set_event_hub(hub).await;
    }

    Ok((manager, transaction_manager, transport))
}

async fn assert_single_options_ok(
    manager: &Arc<DialogManager>,
    transaction_manager: &Arc<TransactionManager>,
    transport: &Arc<MockTransport>,
) -> Response {
    let messages = transport.sent_messages().await;
    assert_eq!(messages.len(), 1, "expected exactly one OPTIONS response");

    let response = match &messages[0].0 {
        Message::Response(response) => response.clone(),
        other => panic!("expected response, got {other:?}"),
    };

    assert_eq!(response.status_code(), 200);
    for header in [
        HeaderName::Via,
        HeaderName::From,
        HeaderName::To,
        HeaderName::CallId,
        HeaderName::CSeq,
        HeaderName::Allow,
        HeaderName::ContentLength,
    ] {
        assert!(
            response.header(&header).is_some(),
            "OPTIONS response missing {header:?}"
        );
    }
    if let Some(TypedHeader::ContentLength(content_length)) =
        response.header(&HeaderName::ContentLength)
    {
        assert_eq!(content_length.0, 0);
    } else {
        panic!("OPTIONS response missing typed Content-Length");
    }
    rvoip_sip_core::validation::validate_wire_response(&response).unwrap();

    let (_, server_transactions) = transaction_manager.active_transactions().await;
    assert_eq!(server_transactions.len(), 1);
    let reached_completed = transaction_manager
        .wait_for_transaction_state(
            &server_transactions[0],
            TransactionState::Completed,
            Duration::from_millis(250),
        )
        .await
        .unwrap();
    let state = transaction_manager
        .transaction_state(&server_transactions[0])
        .await
        .unwrap();
    assert!(
        reached_completed
            || matches!(
                state,
                TransactionState::Completed | TransactionState::Terminated
            ),
        "OPTIONS transaction did not enter a final-response state; current state: {state:?}"
    );
    assert!(
        manager.list_dialogs().is_empty(),
        "OPTIONS must not create dialog state"
    );

    response
}

#[tokio::test]
async fn auto_options_response_sends_valid_200_without_dialog_state(
) -> Result<(), Box<dyn std::error::Error>> {
    let (manager, transaction_manager, transport) =
        manager_with_options_config(true, false).await?;

    manager
        .handle_options(options_request(), "192.0.2.10:5060".parse()?)
        .await?;

    assert_single_options_ok(&manager, &transaction_manager, &transport).await;
    Ok(())
}

#[tokio::test]
async fn options_falls_back_to_200_when_capability_query_is_not_mappable(
) -> Result<(), Box<dyn std::error::Error>> {
    let (manager, transaction_manager, transport) =
        manager_with_options_config(false, true).await?;

    manager
        .handle_options(options_request(), "192.0.2.10:5060".parse()?)
        .await?;

    assert_single_options_ok(&manager, &transaction_manager, &transport).await;
    Ok(())
}
