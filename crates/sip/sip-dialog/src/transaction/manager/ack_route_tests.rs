use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_transport::transport::{TransportAuthority, TransportType};
use rvoip_sip_transport::{Transport, TransportRoute};
use tokio::sync::{mpsc, Mutex};

use super::*;
use crate::transaction::error::{
    Ack2xxFailureStage, Error as TransactionError, Result as TransactionResult,
};

#[derive(Debug)]
struct AckRecordingTransport {
    local_addr: SocketAddr,
    sent: Mutex<Vec<(Message, TransportRoute)>>,
}

impl AckRecordingTransport {
    fn new() -> Self {
        Self {
            local_addr: "127.0.0.1:5070".parse().expect("valid local address"),
            sent: Mutex::new(Vec::new()),
        }
    }

    async fn last_ack(&self) -> (Request, TransportRoute) {
        self.sent
            .lock()
            .await
            .iter()
            .rev()
            .find_map(|(message, route)| match message {
                Message::Request(request) if request.method() == Method::Ack => {
                    Some((request.clone(), route.clone()))
                }
                _ => None,
            })
            .expect("ACK send")
    }
}

#[async_trait]
impl Transport for AckRecordingTransport {
    fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
        Ok(self.local_addr)
    }

    async fn send_message(
        &self,
        message: Message,
        destination: SocketAddr,
    ) -> std::result::Result<(), rvoip_sip_transport::Error> {
        self.sent
            .lock()
            .await
            .push((message, TransportRoute::new(destination)));
        Ok(())
    }

    async fn send_message_via(
        &self,
        message: Message,
        route: TransportRoute,
    ) -> std::result::Result<(), rvoip_sip_transport::Error> {
        self.sent.lock().await.push((message, route));
        Ok(())
    }

    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }

    fn supports_tcp(&self) -> bool {
        true
    }

    fn default_transport_type(&self) -> TransportType {
        TransportType::Tcp
    }
}

fn invite(branch: &str) -> Request {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@192.0.2.20:5060;transport=tcp")
        .expect("request URI")
        .from("Alice", "sip:alice@example.test", Some("alice-tag"))
        .to("Bob", "sip:bob@example.test", None)
        .contact("sip:alice@127.0.0.1:5070;transport=tcp", None)
        .call_id(branch)
        .cseq(101)
        .via("127.0.0.1:5070", "TCP", Some(branch))
        .max_forwards(70)
        .build()
}

fn response(invite: &Request, record_route: Option<&str>) -> Response {
    let mut response =
        SimpleResponseBuilder::response_from_request(invite, StatusCode::Ok, Some("OK"))
            .to("Bob", "sip:bob@example.test", Some("bob-tag"))
            .contact("sip:bob@192.0.2.30:5090;transport=tcp", None)
            .build();
    if let Some(value) = record_route {
        response.headers.push(TypedHeader::RecordRoute(
            RecordRoute::from_str(value).expect("Record-Route"),
        ));
    }
    response
}

async fn manager_with_invite(
    branch: &str,
) -> TransactionResult<(
    TransactionManager,
    Arc<AckRecordingTransport>,
    TransactionKey,
    Request,
)> {
    let transport = Arc::new(AckRecordingTransport::new());
    let (_transport_tx, transport_rx) = mpsc::channel(16);
    let (manager, _events) =
        TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;
    let request = invite(branch);
    let destination = "192.0.2.20:5060".parse().expect("INVITE destination");
    let route = TransportRoute::new(destination)
        .with_transport_type(TransportType::Tcp)
        .with_authority(TransportAuthority::ip(IpAddr::V4(Ipv4Addr::new(
            192, 0, 2, 20,
        ))));
    let transaction = manager
        .create_client_transaction_on_route(request.clone(), route)
        .await?;
    manager.send_request(&transaction).await?;
    Ok((manager, transport, transaction, request))
}

fn route_values(request: &Request) -> Vec<String> {
    request
        .headers
        .iter()
        .filter_map(|header| match header {
            TypedHeader::Route(route) => Some(route.to_string()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn ack_without_route_set_uses_contact_target() -> TransactionResult<()> {
    let (manager, transport, transaction, request) =
        manager_with_invite("z9hG4bK.ack-no-route").await?;

    manager
        .send_ack_for_2xx(&transaction, &response(&request, None))
        .await?;
    let (ack, route) = transport.last_ack().await;
    assert_eq!(
        ack.uri().to_string(),
        "sip:bob@192.0.2.30:5090;transport=tcp"
    );
    assert!(route_values(&ack).is_empty());
    assert_eq!(
        route.destination,
        "192.0.2.30:5090".parse().expect("ACK target")
    );

    manager.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn ack_with_one_record_route_uses_that_first_hop() -> TransactionResult<()> {
    let (manager, transport, transaction, request) =
        manager_with_invite("z9hG4bK.ack-one-route").await?;
    let record_route = "<sip:192.0.2.40:5080;transport=tcp;lr>";

    manager
        .send_ack_for_2xx(&transaction, &response(&request, Some(record_route)))
        .await?;
    let (ack, route) = transport.last_ack().await;
    assert_eq!(route_values(&ack), vec![record_route.to_string()]);
    assert_eq!(
        route.destination,
        "192.0.2.40:5080".parse().expect("proxy target")
    );
    assert_eq!(route.transport_type, Some(TransportType::Tcp));
    assert_eq!(
        route.authority,
        Some(TransportAuthority::ip(IpAddr::V4(Ipv4Addr::new(
            192, 0, 2, 40,
        ))))
    );

    manager.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn ack_with_two_record_routes_reverses_the_uac_route_set() -> TransactionResult<()> {
    let (manager, transport, transaction, request) =
        manager_with_invite("z9hG4bK.ack-two-routes").await?;
    let first = "<sip:192.0.2.41:5081;transport=tcp;lr>";
    let second = "<sip:192.0.2.42:5082;transport=tcp;lr>";
    let record_route = format!("{first}, {second}");

    manager
        .send_ack_for_2xx(&transaction, &response(&request, Some(&record_route)))
        .await?;
    let (ack, route) = transport.last_ack().await;
    assert_eq!(
        route_values(&ack),
        vec![second.to_string(), first.to_string()]
    );
    assert_eq!(
        route.destination,
        "192.0.2.42:5082".parse().expect("first hop")
    );

    manager.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn ack_preserves_long_record_route_uri_parameters() -> TransactionResult<()> {
    let (manager, transport, transaction, request) =
        manager_with_invite("z9hG4bK.ack-long-route").await?;
    let token = "a".repeat(700);
    let record_route =
        format!("<sip:192.0.2.43:5083;transport=tcp;lr;esp={token};espv=signed-{token}>");

    manager
        .send_ack_for_2xx(&transaction, &response(&request, Some(&record_route)))
        .await?;
    let (ack, route) = transport.last_ack().await;
    assert_eq!(
        ack.uri().to_string(),
        "sip:bob@192.0.2.30:5090;transport=tcp"
    );
    assert_eq!(route_values(&ack), vec![record_route]);
    assert_eq!(
        route.destination,
        "192.0.2.43:5083".parse().expect("first hop")
    );

    manager.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn missing_invite_route_fails_immediately_with_a_distinct_stage() -> TransactionResult<()> {
    let transport = Arc::new(AckRecordingTransport::new());
    let (_transport_tx, transport_rx) = mpsc::channel(16);
    let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(16)).await?;
    let request = invite("z9hG4bK.ack-missing-route");
    let missing = TransactionKey::new("z9hG4bK.ack-missing-route".into(), Method::Invite, false);

    let error = tokio::time::timeout(
        Duration::from_millis(100),
        manager.send_ack_for_2xx(&missing, &response(&request, None)),
    )
    .await
    .expect("route lookup must not wait")
    .expect_err("missing route must fail");
    assert!(matches!(
        error,
        TransactionError::Ack2xxFailure {
            stage: Ack2xxFailureStage::RouteLookup,
            ..
        }
    ));

    manager.shutdown().await;
    Ok(())
}
