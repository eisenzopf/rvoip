//! Live transaction-boundary evidence for RFC 3261 §16 routing.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::content_length::ContentLength;
use rvoip_sip_core::types::headers::{HeaderName, HeaderValue};
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::proxy_require::ProxyRequire;
use rvoip_sip_core::types::record_route::RecordRoute;
use rvoip_sip_core::types::route::Route;
use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{parse_message, Message, Method, Request, Uri};
use rvoip_sip_dialog::transaction::TransactionManager;
use rvoip_sip_proxy::{
    ForkMode, ProxyConfig, ProxyRoutingPolicy, ProxyRuntimeOptions, ProxyTarget, RecordRoutePolicy,
    RedirectDecision, RedirectInfo, RedirectInterceptor, RouteDecision, RouteFn, StatefulProxy,
    UriRouteDecision, UriRouteFn,
};
use rvoip_sip_transport::resolver::{ResolvedTarget, Resolver, ResolverError};
use rvoip_sip_transport::transport::{TransportRoute, TransportType};
use rvoip_sip_transport::{Transport, TransportEvent};
use tokio::sync::{mpsc, Mutex};

const PROXY_ADDR: &str = "127.0.0.1:5060";
const UAC_ADDR: &str = "10.0.0.5:5060";

#[derive(Debug, Clone)]
struct RoutedMockTransport {
    local_addr: SocketAddr,
    sent: Arc<Mutex<Vec<(Message, TransportRoute)>>>,
    fail: Arc<Mutex<Vec<SocketAddr>>>,
}

impl RoutedMockTransport {
    fn new() -> Self {
        Self {
            local_addr: PROXY_ADDR.parse().unwrap(),
            sent: Arc::new(Mutex::new(Vec::new())),
            fail: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn fail_for(&self, address: SocketAddr) {
        self.fail.lock().await.push(address);
    }

    async fn sent(&self) -> Vec<(Message, TransportRoute)> {
        self.sent.lock().await.clone()
    }
}

#[async_trait]
impl Transport for RoutedMockTransport {
    async fn send_message(
        &self,
        message: Message,
        destination: SocketAddr,
    ) -> Result<(), rvoip_sip_transport::Error> {
        self.send_message_via(message, TransportRoute::new(destination))
            .await
    }

    async fn send_message_via(
        &self,
        message: Message,
        route: TransportRoute,
    ) -> Result<(), rvoip_sip_transport::Error> {
        self.sent.lock().await.push((message, route.clone()));
        if self.fail.lock().await.contains(&route.destination) {
            return Err(rvoip_sip_transport::Error::ConnectFailed(
                route.destination,
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "programmed failure"),
            ));
        }
        Ok(())
    }

    fn local_addr(&self) -> Result<SocketAddr, rvoip_sip_transport::Error> {
        Ok(self.local_addr)
    }

    fn supports_tcp(&self) -> bool {
        true
    }

    fn supports_tls(&self) -> bool {
        true
    }

    async fn close(&self) -> Result<(), rvoip_sip_transport::Error> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct CannedResolver {
    responses: Mutex<HashMap<String, Vec<ResolvedTarget>>>,
    calls: Mutex<Vec<String>>,
}

impl CannedResolver {
    async fn set(&self, uri: &str, targets: Vec<ResolvedTarget>) {
        self.responses.lock().await.insert(uri.into(), targets);
    }

    async fn calls(&self) -> Vec<String> {
        self.calls.lock().await.clone()
    }
}

#[async_trait]
impl Resolver for CannedResolver {
    async fn resolve(&self, uri: &Uri) -> Result<Vec<ResolvedTarget>, ResolverError> {
        let key = uri.to_string();
        self.calls.lock().await.push(key.clone());
        self.responses
            .lock()
            .await
            .get(&key)
            .cloned()
            .ok_or(ResolverError::NoCandidates)
    }
}

struct Harness {
    transport: Arc<RoutedMockTransport>,
    ingress: mpsc::Sender<TransportEvent>,
    _tm: Arc<TransactionManager>,
    proxy: Arc<StatefulProxy>,
    _task: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new(route_fn: RouteFn, config: ProxyConfig, options: ProxyRuntimeOptions) -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("rvoip_sip_proxy=trace,rvoip_sip_dialog=warn")
            .with_test_writer()
            .try_init();
        let transport = Arc::new(RoutedMockTransport::new());
        let (ingress, events_rx) = mpsc::channel(64);
        let (tm, proxy_events) = TransactionManager::new(transport.clone(), events_rx, Some(32))
            .await
            .unwrap();
        let tm = Arc::new(tm);
        let proxy = StatefulProxy::with_options(tm.clone(), route_fn, config, options);
        let task = proxy.clone().run(proxy_events);
        Self {
            transport,
            ingress,
            _tm: tm,
            proxy,
            _task: task,
        }
    }

    async fn new_uri(
        route_fn: UriRouteFn,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
    ) -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("rvoip_sip_proxy=trace,rvoip_sip_dialog=warn")
            .with_test_writer()
            .try_init();
        let transport = Arc::new(RoutedMockTransport::new());
        let (ingress, events_rx) = mpsc::channel(64);
        let (tm, proxy_events) = TransactionManager::new(transport.clone(), events_rx, Some(32))
            .await
            .unwrap();
        let tm = Arc::new(tm);
        let proxy = StatefulProxy::with_uri_routes(tm.clone(), route_fn, config, options);
        let task = proxy.clone().run(proxy_events);
        Self {
            transport,
            ingress,
            _tm: tm,
            proxy,
            _task: task,
        }
    }

    async fn inject(&self, request: Request) {
        self.inject_message(
            Message::Request(request),
            UAC_ADDR.parse().unwrap(),
            TransportType::Udp,
        )
        .await;
    }

    async fn inject_message(
        &self,
        message: Message,
        source: SocketAddr,
        transport_type: TransportType,
    ) {
        self.ingress
            .send(TransportEvent::MessageReceived {
                message,
                source,
                destination: self.transport.local_addr,
                transport_type,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await
            .unwrap();
    }

    async fn wait_until<F>(&self, predicate: F) -> Vec<(Message, TransportRoute)>
    where
        F: Fn(&[(Message, TransportRoute)]) -> bool,
    {
        let started = std::time::Instant::now();
        loop {
            let sent = self.transport.sent().await;
            if predicate(&sent) {
                return sent;
            }
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "timed out; sent={sent:#?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

struct FixedRedirectTargets {
    targets: Vec<SocketAddr>,
}

#[async_trait]
impl RedirectInterceptor for FixedRedirectTargets {
    async fn on_redirect(&self, _info: RedirectInfo) -> Option<RedirectDecision> {
        Some(RedirectDecision::ReFork {
            mode: ForkMode::Sequential,
            targets: self.targets.clone(),
        })
    }
}

fn uri(value: &str) -> Uri {
    Uri::from_str(value).unwrap()
}

fn invite(call_id: &str, target: &str) -> Request {
    request(Method::Invite, call_id, target)
}

fn request(method: Method, call_id: &str, target: &str) -> Request {
    SimpleRequestBuilder::new(method, target)
        .unwrap()
        .from("Alice", "sip:alice@uac.example", Some("from-tag"))
        .to("Agent", target, None)
        .call_id(call_id)
        .cseq(1)
        .contact("sip:alice@10.0.0.5:5060", None)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch(format!("z9hG4bK-{call_id}"))],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

fn request_sends(sent: &[(Message, TransportRoute)]) -> Vec<(&Request, &TransportRoute)> {
    sent.iter()
        .filter_map(|(message, route)| match message {
            Message::Request(request) => Some((request, route)),
            Message::Response(_) => None,
        })
        .collect()
}

#[tokio::test]
async fn loose_route_is_preprocessed_then_resolved_with_ordered_failover() {
    let resolver = Arc::new(CannedResolver::default());
    let first: SocketAddr = "10.0.0.20:5070".parse().unwrap();
    let second: SocketAddr = "10.0.0.21:5070".parse().unwrap();
    resolver
        .set(
            "sip:edge.example:5070;transport=tcp;lr",
            vec![
                ResolvedTarget::immediate(first, TransportType::Tcp),
                ResolvedTarget::immediate(second, TransportType::Tcp),
            ],
        )
        .await;

    let target = ProxyTarget::new(uri("sip:agent@callcenter.example"));
    let route_fn: UriRouteFn = Arc::new(move |_| Some(UriRouteDecision::to(target.clone())));
    let routing = ProxyRoutingPolicy::new(vec![uri("sip:proxy.example:5060;lr")]).unwrap();
    let options = ProxyRuntimeOptions::default()
        .with_resolver(resolver.clone())
        .with_routing_policy(routing)
        .with_advertised_via(TransportType::Tcp, "198.51.100.10:5091".parse().unwrap());
    let harness = Harness::new_uri(route_fn, ProxyConfig::default(), options).await;
    harness.transport.fail_for(first).await;

    let mut request = invite("loose-route", "sip:original@example.invalid");
    request.headers.push(TypedHeader::Route(
        Route::from_str("<sip:proxy.example:5060;lr>, <sip:edge.example:5070;transport=tcp;lr>")
            .unwrap(),
    ));
    harness.inject(request).await;

    let sent = harness
        .wait_until(|sent| {
            let requests = request_sends(sent);
            requests.iter().any(|(_, route)| route.destination == first)
                && requests
                    .iter()
                    .any(|(_, route)| route.destination == second)
        })
        .await;
    assert_eq!(
        resolver.calls().await,
        vec!["sip:edge.example:5070;transport=tcp;lr"]
    );
    let requests = request_sends(&sent);
    let (forwarded, route) = requests
        .iter()
        .find(|(_, route)| route.destination == second)
        .copied()
        .unwrap();
    assert_eq!(forwarded.uri(), &uri("sip:agent@callcenter.example"));
    let route_text = forwarded
        .headers
        .iter()
        .find(|header| matches!(header, TypedHeader::Route(_)))
        .unwrap()
        .to_string();
    assert!(!route_text.contains("proxy.example"));
    assert!(route_text.contains("edge.example"));
    assert_eq!(route.transport_type, Some(TransportType::Tcp));
    let via = &forwarded.via_headers()[0].0[0];
    assert_eq!(via.transport(), "TCP");
    assert_eq!(via.sent_by_port, Some(5091));
}

#[tokio::test]
async fn strict_route_rewrites_request_uri_and_appends_original_target_live() {
    let resolver = Arc::new(CannedResolver::default());
    let strict_addr: SocketAddr = "10.0.0.30:5060".parse().unwrap();
    resolver
        .set(
            "sip:strict-router.example:5060",
            vec![ResolvedTarget::immediate(strict_addr, TransportType::Udp)],
        )
        .await;
    let target = ProxyTarget::new(uri("sip:agent@callcenter.example"))
        .with_route_set(vec![uri("sip:strict-router.example:5060")]);
    let route_fn: UriRouteFn = Arc::new(move |_| Some(UriRouteDecision::to(target.clone())));
    let options = ProxyRuntimeOptions::default()
        .with_resolver(resolver)
        .with_advertised_via(TransportType::Udp, "198.51.100.10:5080".parse().unwrap());
    let harness = Harness::new_uri(route_fn, ProxyConfig::default(), options).await;
    harness
        .inject(invite("strict-route", "sip:original@example.invalid"))
        .await;

    let sent = harness
        .wait_until(|sent| {
            request_sends(sent)
                .iter()
                .any(|(_, route)| route.destination == strict_addr)
        })
        .await;
    let (forwarded, _) = request_sends(&sent)
        .into_iter()
        .find(|(_, route)| route.destination == strict_addr)
        .unwrap();
    assert_eq!(forwarded.uri(), &uri("sip:strict-router.example:5060"));
    assert!(forwarded
        .headers
        .iter()
        .filter(|header| matches!(header, TypedHeader::Route(_)))
        .any(|header| header.to_string().contains("agent@callcenter.example")));
}

#[tokio::test]
async fn unsupported_proxy_require_returns_complete_420_without_routing() {
    let route_calls = Arc::new(AtomicUsize::new(0));
    let route_calls_copy = route_calls.clone();
    let route_fn: RouteFn = Arc::new(move |_| {
        route_calls_copy.fetch_add(1, Ordering::Relaxed);
        Some(RouteDecision::to("10.0.0.50:5060".parse().unwrap()))
    });
    let routing =
        ProxyRoutingPolicy::default().with_supported_proxy_require(["supported-extension"]);
    let harness = Harness::new(
        route_fn,
        ProxyConfig::default(),
        ProxyRuntimeOptions::default()
            .with_routing_policy(routing)
            .with_resolver(Arc::new(CannedResolver::default())),
    )
    .await;

    let request = invite("proxy-require", "sip:agent@example.net")
        .with_header(TypedHeader::ProxyRequire(ProxyRequire::with_options(&[
            "foo",
            "supported-extension",
        ])))
        .with_header(TypedHeader::ProxyRequire(ProxyRequire::with_options(&[
            "bar", "FOO",
        ])));
    harness.inject(request).await;
    let sent = harness
        .wait_until(|sent| {
            sent.iter().any(
                |(message, _)| matches!(message, Message::Response(response) if response.status() == StatusCode::BadExtension),
            )
        })
        .await;
    assert_eq!(route_calls.load(Ordering::Relaxed), 0);
    let unsupported = sent
        .iter()
        .find_map(|(message, _)| match message {
            Message::Response(response) if response.status() == StatusCode::BadExtension => {
                response.headers.iter().find_map(|header| match header {
                    TypedHeader::Unsupported(value) => Some(value.option_tags().to_vec()),
                    _ => None,
                })
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(unsupported, vec!["foo".to_string(), "bar".to_string()]);
}

#[tokio::test]
async fn unsupported_request_uri_scheme_returns_416_without_routing() {
    let route_calls = Arc::new(AtomicUsize::new(0));
    let route_calls_copy = route_calls.clone();
    let route_fn: RouteFn = Arc::new(move |_| {
        route_calls_copy.fetch_add(1, Ordering::Relaxed);
        Some(RouteDecision::to("10.0.0.50:5060".parse().unwrap()))
    });
    let harness = Harness::new(
        route_fn,
        ProxyConfig::default(),
        ProxyRuntimeOptions::default().with_resolver(Arc::new(CannedResolver::default())),
    )
    .await;
    harness
        .inject(invite("unsupported-uri", "tel:+12065550100"))
        .await;

    let sent = harness
        .wait_until(|sent| {
            sent.iter().any(
                |(message, _)| matches!(message, Message::Response(response) if response.status() == StatusCode::UnsupportedUriScheme),
            )
        })
        .await;
    assert_eq!(route_calls.load(Ordering::Relaxed), 0);
    assert!(sent.iter().any(
        |(message, _)| matches!(message, Message::Response(response) if response.status() == StatusCode::UnsupportedUriScheme)
    ));
}

#[tokio::test]
async fn per_transport_advertised_via_and_sips_no_downgrade_are_enforced() {
    let resolver = Arc::new(CannedResolver::default());
    let udp_addr: SocketAddr = "10.0.0.60:5060".parse().unwrap();
    let tcp_addr: SocketAddr = "10.0.0.61:5060".parse().unwrap();
    let insecure_sips_addr: SocketAddr = "10.0.0.62:5060".parse().unwrap();
    let tls_addr: SocketAddr = "10.0.0.63:5061".parse().unwrap();
    resolver
        .set(
            "sip:udp.example;transport=udp",
            vec![ResolvedTarget::immediate(udp_addr, TransportType::Udp)],
        )
        .await;
    resolver
        .set(
            "sip:tcp.example;transport=tcp",
            vec![ResolvedTarget::immediate(tcp_addr, TransportType::Tcp)],
        )
        .await;
    resolver
        .set(
            "sips:secure.example",
            vec![
                ResolvedTarget::immediate(insecure_sips_addr, TransportType::Udp),
                ResolvedTarget::immediate(tls_addr, TransportType::Tls),
            ],
        )
        .await;

    let targets = vec![
        ProxyTarget::new(uri("sip:udp.example;transport=udp")),
        ProxyTarget::new(uri("sip:tcp.example;transport=tcp")),
        ProxyTarget::new(uri("sips:secure.example")),
    ];
    let route_fn: UriRouteFn = Arc::new(move |_| Some(UriRouteDecision::parallel(targets.clone())));
    let options = ProxyRuntimeOptions::default()
        .with_resolver(resolver)
        .with_advertised_via(TransportType::Udp, "198.51.100.10:5080".parse().unwrap())
        .with_advertised_via(TransportType::Tcp, "198.51.100.10:5081".parse().unwrap())
        .with_advertised_via(TransportType::Tls, "198.51.100.10:5082".parse().unwrap());
    let harness = Harness::new_uri(route_fn, ProxyConfig::default(), options).await;
    harness
        .inject(invite("transport-vias", "sip:original@example.invalid"))
        .await;

    let sent = harness
        .wait_until(|sent| {
            let routes = request_sends(sent);
            [udp_addr, tcp_addr, tls_addr].iter().all(|destination| {
                routes
                    .iter()
                    .any(|(_, route)| route.destination == *destination)
            })
        })
        .await;
    let requests = request_sends(&sent);
    for (destination, transport, token, port) in [
        (udp_addr, TransportType::Udp, "UDP", 5080),
        (tcp_addr, TransportType::Tcp, "TCP", 5081),
        (tls_addr, TransportType::Tls, "TLS", 5082),
    ] {
        let (request, route) = requests
            .iter()
            .find(|(_, route)| route.destination == destination)
            .copied()
            .unwrap();
        assert_eq!(route.transport_type, Some(transport));
        let via = &request.via_headers()[0].0[0];
        assert_eq!(via.transport(), token);
        assert_eq!(via.sent_by_port, Some(port));
    }
    assert!(
        requests
            .iter()
            .all(|(_, route)| route.destination != insecure_sips_addr),
        "an insecure resolver candidate must never carry a SIPS request"
    );
}

#[tokio::test]
async fn stateless_ack_and_unmatched_cancel_share_uri_routing_and_exact_via() {
    let resolver = Arc::new(CannedResolver::default());
    let destination: SocketAddr = "10.0.0.70:5070".parse().unwrap();
    resolver
        .set(
            "sip:stateless.example:5070;transport=tcp",
            vec![ResolvedTarget::immediate(destination, TransportType::Tcp)],
        )
        .await;
    let target = ProxyTarget::new(uri("sip:stateless.example:5070;transport=tcp"));
    let route_fn: UriRouteFn = Arc::new(move |_| Some(UriRouteDecision::to(target.clone())));
    let options = ProxyRuntimeOptions::default()
        .with_resolver(resolver.clone())
        .with_advertised_via(TransportType::Tcp, "198.51.100.10:5099".parse().unwrap());
    let harness = Harness::new_uri(route_fn, ProxyConfig::default(), options).await;

    harness
        .inject(request(
            Method::Ack,
            "stateless-ack",
            "sip:original@example.invalid",
        ))
        .await;
    harness
        .inject(
            request(
                Method::Cancel,
                "unmatched-cancel",
                "sip:original@example.invalid",
            )
            .with_header(TypedHeader::ProxyRequire(ProxyRequire::single(
                "unknown-but-ignored-for-cancel",
            ))),
        )
        .await;

    let sent = harness
        .wait_until(|sent| {
            let requests = request_sends(sent);
            [Method::Ack, Method::Cancel].iter().all(|method| {
                requests.iter().any(|(request, route)| {
                    request.method() == *method && route.destination == destination
                })
            })
        })
        .await;
    for (request, route) in request_sends(&sent)
        .into_iter()
        .filter(|(_, route)| route.destination == destination)
    {
        assert_eq!(route.transport_type, Some(TransportType::Tcp));
        let via = &request.via_headers()[0].0[0];
        assert_eq!(via.transport(), "TCP");
        assert_eq!(via.sent_by_port, Some(5099));
    }
    assert_eq!(resolver.calls().await.len(), 2);
}

#[tokio::test]
async fn target_copy_preserves_body_header_order_and_content_length_live() {
    const BODY: &[u8] = b"\0binary\r\nproxy-body\xff";
    const EXTENSION: &str = "X-Rvoip-Order";
    let destination: SocketAddr = "10.0.0.80:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_| Some(RouteDecision::to(destination)));
    let harness = Harness::new(
        route_fn,
        ProxyConfig::default(),
        ProxyRuntimeOptions::default(),
    )
    .await;

    let mut message = SimpleRequestBuilder::new(Method::Message, "sip:agent@example.net")
        .unwrap()
        .from("Alice", "sip:alice@uac.example", Some("from-tag"))
        .to("Agent", "sip:agent@example.net", None)
        .call_id("request-copy-live")
        .cseq(1)
        .contact("sip:alice@10.0.0.5:5060", None)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch("z9hG4bK-request-copy-live")],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .content_type("application/octet-stream")
        .body(BODY)
        .build();
    message.headers.push(TypedHeader::Other(
        HeaderName::Other(EXTENSION.into()),
        HeaderValue::Raw(b"first".to_vec()),
    ));
    message.headers.push(TypedHeader::Other(
        HeaderName::Other(EXTENSION.into()),
        HeaderValue::Raw(b"second".to_vec()),
    ));
    harness.inject(message).await;

    let sent = harness
        .wait_until(|sent| {
            request_sends(sent)
                .iter()
                .any(|(_, route)| route.destination == destination)
        })
        .await;
    let (forwarded, _) = request_sends(&sent)
        .into_iter()
        .find(|(_, route)| route.destination == destination)
        .unwrap();
    assert_eq!(forwarded.body(), BODY);
    let extension_values: Vec<_> = forwarded
        .headers
        .iter()
        .filter_map(|header| match header {
            TypedHeader::Other(name, HeaderValue::Raw(value))
                if name.wire_eq(&HeaderName::Other(EXTENSION.into())) =>
            {
                Some(value.as_slice())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        extension_values,
        vec![b"first".as_slice(), b"second".as_slice()]
    );
    assert!(forwarded.headers.iter().any(
        |header| matches!(header, TypedHeader::ContentLength(value) if value.0 == BODY.len() as u32)
    ));
}

#[tokio::test]
async fn strict_router_inbound_recovery_uses_last_route_then_preserves_remaining_set_live() {
    let resolver = Arc::new(CannedResolver::default());
    let next_hop: SocketAddr = "10.0.0.90:5060".parse().unwrap();
    resolver
        .set(
            "sip:next.example.net:5060;lr",
            vec![ResolvedTarget::immediate(next_hop, TransportType::Udp)],
        )
        .await;
    let route_fn: UriRouteFn = Arc::new(|request| {
        Some(UriRouteDecision::to(ProxyTarget::new(
            request.uri().clone(),
        )))
    });
    let record_route = RecordRoutePolicy::new(
        uri("sip:proxy.example:5060;lr"),
        uri("sips:proxy.example:5061;lr"),
    )
    .unwrap();
    let routing = ProxyRoutingPolicy::new(vec![uri("sip:proxy.example:5060;lr")])
        .unwrap()
        .with_record_route(record_route);
    let options = ProxyRuntimeOptions::default()
        .with_resolver(resolver)
        .with_routing_policy(routing)
        .with_advertised_via(TransportType::Udp, "198.51.100.10:5080".parse().unwrap());
    let harness = Harness::new_uri(route_fn, ProxyConfig::default(), options).await;

    let mut inbound = invite("strict-inbound-live", "sip:proxy.example:5060");
    inbound.headers.push(TypedHeader::Route(
        Route::from_str("<sip:next.example.net:5060;lr>").unwrap(),
    ));
    inbound.headers.push(TypedHeader::Route(
        Route::from_str("<sip:agent@destination.example>").unwrap(),
    ));
    harness.inject(inbound).await;

    let sent = harness
        .wait_until(|sent| {
            request_sends(sent)
                .iter()
                .any(|(_, route)| route.destination == next_hop)
        })
        .await;
    let (forwarded, _) = request_sends(&sent)
        .into_iter()
        .find(|(_, route)| route.destination == next_hop)
        .unwrap();
    assert_eq!(forwarded.uri(), &uri("sip:agent@destination.example"));
    let route_values: Vec<_> = forwarded
        .headers
        .iter()
        .filter_map(|header| match header {
            TypedHeader::Route(route) => Some(route.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        route_values,
        vec!["<sip:next.example.net:5060;lr>".to_string()]
    );
}

#[tokio::test]
async fn secure_record_route_is_prepended_without_reordering_existing_values_live() {
    let resolver = Arc::new(CannedResolver::default());
    let destination: SocketAddr = "10.0.0.100:5061".parse().unwrap();
    resolver
        .set(
            "sips:agent@secure.example",
            vec![ResolvedTarget::immediate(destination, TransportType::Tls)],
        )
        .await;
    let target = ProxyTarget::new(uri("sips:agent@secure.example"));
    let route_fn: UriRouteFn = Arc::new(move |_| Some(UriRouteDecision::to(target.clone())));
    let record_route = RecordRoutePolicy::new(
        uri("sip:proxy.example:5060;lr"),
        uri("sips:proxy.example:5061;lr"),
    )
    .unwrap();
    let routing = ProxyRoutingPolicy::new(vec![
        uri("sip:proxy.example:5060;lr"),
        uri("sips:proxy.example:5061;lr"),
    ])
    .unwrap()
    .with_record_route(record_route);
    let options = ProxyRuntimeOptions::default()
        .with_resolver(resolver)
        .with_routing_policy(routing)
        .with_advertised_via(TransportType::Tls, "198.51.100.10:5082".parse().unwrap());
    let harness = Harness::new_uri(route_fn, ProxyConfig::default(), options).await;

    let mut inbound = invite("record-route-live", "sips:service@example.net");
    inbound.headers.push(TypedHeader::RecordRoute(
        RecordRoute::from_str("<sips:old.example.net;lr>, <sips:older.example.net;lr>").unwrap(),
    ));
    harness.inject(inbound).await;

    let sent = harness
        .wait_until(|sent| {
            request_sends(sent)
                .iter()
                .any(|(_, route)| route.destination == destination)
        })
        .await;
    let (forwarded, exact_route) = request_sends(&sent)
        .into_iter()
        .find(|(_, route)| route.destination == destination)
        .unwrap();
    assert_eq!(exact_route.transport_type, Some(TransportType::Tls));
    assert_eq!(forwarded.uri(), &uri("sips:agent@secure.example"));
    let record_routes: Vec<_> = forwarded
        .headers
        .iter()
        .filter_map(|header| match header {
            TypedHeader::RecordRoute(value) => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(record_routes.len(), 2);
    assert_eq!(
        record_routes[0].first().unwrap().uri(),
        &uri("sips:proxy.example:5061;lr")
    );
    assert_eq!(record_routes[1].len(), 2);
    assert_eq!(record_routes[1].0[0].uri(), &uri("sips:old.example.net;lr"));
    assert_eq!(
        record_routes[1].0[1].uri(),
        &uri("sips:older.example.net;lr")
    );
    assert!(record_routes[1].0.iter().all(|entry| entry.has_param("lr")));
}

#[tokio::test]
async fn unknown_method_and_malformed_unrelated_header_are_forwarded_unchanged() {
    let destination: SocketAddr = "10.0.0.110:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_| Some(RouteDecision::to(destination)));
    let harness = Harness::new(
        route_fn,
        ProxyConfig::default(),
        ProxyRuntimeOptions::default(),
    )
    .await;
    let wire = b"SERVICE sip:agent@example.net SIP/2.0\r\n\
Via: SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bK-extension-live\r\n\
From: <sip:alice@example.net>;tag=from-tag\r\n\
To: <sip:agent@example.net>\r\n\
Call-ID: extension-live\r\n\
CSeq: 1 SERVICE\r\n\
Max-Forwards: 70\r\n\
Date: definitely-not-an-rfc-date\r\n\
X-Unrelated-Extension: [opaque@@value\r\n\
Content-Length: 0\r\n\r\n";
    let Message::Request(request) = parse_message(wire).expect("tolerant SIP parser") else {
        unreachable!();
    };
    assert!(matches!(
        request.method(),
        Method::Extension(method) if method == "SERVICE"
    ));
    harness.inject(request).await;

    let sent = harness
        .wait_until(|sent| {
            request_sends(sent)
                .iter()
                .any(|(_, route)| route.destination == destination)
        })
        .await;
    let (forwarded, _) = request_sends(&sent)
        .into_iter()
        .find(|(_, route)| route.destination == destination)
        .unwrap();
    assert!(matches!(
        forwarded.method(),
        Method::Extension(method) if method == "SERVICE"
    ));
    assert!(forwarded.headers.iter().any(|header| {
        matches!(
            header,
            TypedHeader::Other(HeaderName::Date, HeaderValue::Raw(value))
                if value == b"definitely-not-an-rfc-date"
        )
    }));
    assert!(forwarded.headers.iter().any(|header| {
        matches!(
            header,
            TypedHeader::Other(name, HeaderValue::Raw(value))
                if name.wire_eq(&HeaderName::Other("X-Unrelated-Extension".into()))
                    && value == b"[opaque@@value"
        )
    }));
}

#[tokio::test]
async fn known_resource_with_empty_target_set_returns_480_without_creating_a_leg() {
    let route_calls = Arc::new(AtomicUsize::new(0));
    let route_calls_copy = route_calls.clone();
    let route_fn: RouteFn = Arc::new(move |_| {
        route_calls_copy.fetch_add(1, Ordering::Relaxed);
        Some(RouteDecision::parallel(Vec::new()))
    });
    let harness = Harness::new(
        route_fn,
        ProxyConfig::default(),
        ProxyRuntimeOptions::default(),
    )
    .await;
    harness
        .inject(invite("empty-target-set", "sip:known-resource@example.net"))
        .await;

    let sent = harness
        .wait_until(|sent| {
            sent.iter().any(|(message, _)| {
                matches!(
                    message,
                    Message::Response(response)
                        if response.status() == StatusCode::TemporarilyUnavailable
                )
            })
        })
        .await;
    assert_eq!(route_calls.load(Ordering::Relaxed), 1);
    assert!(request_sends(&sent).is_empty());
}

#[tokio::test]
async fn recursive_redirect_adds_each_legacy_socket_target_once() {
    let original: SocketAddr = "10.0.0.120:5060".parse().unwrap();
    let backup: SocketAddr = "10.0.0.121:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_| Some(RouteDecision::to(original)));
    let harness = Harness::new(
        route_fn,
        ProxyConfig::default(),
        ProxyRuntimeOptions::default(),
    )
    .await;
    harness
        .proxy
        .set_redirect_interceptor(Some(Arc::new(FixedRedirectTargets {
            targets: vec![original, backup, backup, original],
        })));
    harness
        .inject(invite("recursive-redirect-dedup", "sip:agent@example.net"))
        .await;

    let sent = harness
        .wait_until(|sent| {
            request_sends(sent)
                .iter()
                .any(|(_, route)| route.destination == original)
        })
        .await;
    let (first_leg, _) = request_sends(&sent)
        .into_iter()
        .find(|(_, route)| route.destination == original)
        .unwrap();
    let first_redirect =
        SimpleResponseBuilder::response_from_request(first_leg, StatusCode::MovedTemporarily, None)
            .contact("sip:agent@redirect.example.net", None)
            .build();
    harness
        .inject_message(
            Message::Response(first_redirect),
            original,
            TransportType::Udp,
        )
        .await;

    let sent = harness
        .wait_until(|sent| {
            request_sends(sent)
                .iter()
                .any(|(_, route)| route.destination == backup)
        })
        .await;
    let (backup_leg, _) = request_sends(&sent)
        .into_iter()
        .find(|(_, route)| route.destination == backup)
        .unwrap();
    assert_eq!(
        backup_leg.headers.iter().find_map(|header| match header {
            TypedHeader::MaxForwards(value) => Some(value.0),
            _ => None,
        }),
        Some(69),
        "redirect recursion must not restart Max-Forwards processing"
    );
    let second_redirect = SimpleResponseBuilder::response_from_request(
        backup_leg,
        StatusCode::MovedTemporarily,
        None,
    )
    .contact("sip:agent@redirect.example.net", None)
    .build();
    harness
        .inject_message(
            Message::Response(second_redirect),
            backup,
            TransportType::Udp,
        )
        .await;

    let sent = harness
        .wait_until(|sent| {
            sent.iter().any(|(message, route)| {
                matches!(
                    message,
                    Message::Response(response)
                        if response.status() == StatusCode::MovedTemporarily
                ) && route.destination == UAC_ADDR.parse::<SocketAddr>().unwrap()
            })
        })
        .await;
    let sends = request_sends(&sent);
    assert_eq!(
        sends
            .iter()
            .filter(|(request, route)| {
                request.method() == Method::Invite && route.destination == original
            })
            .count(),
        1
    );
    assert_eq!(
        sends
            .iter()
            .filter(|(request, route)| {
                request.method() == Method::Invite && route.destination == backup
            })
            .count(),
        1
    );
}

#[tokio::test]
async fn consumed_redirect_does_not_compete_with_the_reforked_final_response() {
    let original: SocketAddr = "10.0.0.140:5060".parse().unwrap();
    let backup: SocketAddr = "10.0.0.141:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_| Some(RouteDecision::to(original)));
    let harness = Harness::new(
        route_fn,
        ProxyConfig::default(),
        ProxyRuntimeOptions::default(),
    )
    .await;
    harness
        .proxy
        .set_redirect_interceptor(Some(Arc::new(FixedRedirectTargets {
            targets: vec![backup],
        })));
    harness
        .inject(invite("redirect-consumed", "sip:agent@example.net"))
        .await;

    let sent = harness
        .wait_until(|sent| {
            request_sends(sent)
                .iter()
                .any(|(_, route)| route.destination == original)
        })
        .await;
    let (first_leg, _) = request_sends(&sent)
        .into_iter()
        .find(|(request, route)| {
            request.method() == Method::Invite && route.destination == original
        })
        .unwrap();
    let redirect =
        SimpleResponseBuilder::response_from_request(first_leg, StatusCode::MovedTemporarily, None)
            .contact("sip:agent@redirect.example.net", None)
            .build();
    harness
        .inject_message(Message::Response(redirect), original, TransportType::Udp)
        .await;

    let sent = harness
        .wait_until(|sent| {
            request_sends(sent)
                .iter()
                .any(|(_, route)| route.destination == backup)
        })
        .await;
    let (backup_leg, _) = request_sends(&sent)
        .into_iter()
        .find(|(request, route)| request.method() == Method::Invite && route.destination == backup)
        .unwrap();
    let not_found =
        SimpleResponseBuilder::response_from_request(backup_leg, StatusCode::NotFound, None)
            .build();
    harness
        .inject_message(Message::Response(not_found), backup, TransportType::Udp)
        .await;

    let sent = harness
        .wait_until(|sent| {
            sent.iter().any(|(message, route)| {
                matches!(
                    message,
                    Message::Response(response) if response.status() == StatusCode::NotFound
                ) && route.destination == UAC_ADDR.parse::<SocketAddr>().unwrap()
            })
        })
        .await;
    assert!(!sent.iter().any(|(message, route)| {
        matches!(
            message,
            Message::Response(response) if response.status() == StatusCode::MovedTemporarily
        ) && route.destination == UAC_ADDR.parse::<SocketAddr>().unwrap()
    }));
}
