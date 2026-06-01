//! Loopback test per `UCTP_IMPLEMENTATION_PLAN.md` §5.5.
//!
//! Same shape as `rvoip-quic`'s loopback but goes through the
//! WebTransport HTTP/3 + extended-CONNECT upgrade.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_auth_core::bearer_stub;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::auth;
use rvoip_uctp::substrate::{dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::types::MessageType;
use rvoip_webtransport::{UctpWtAdapter, UctpWtClient, UctpWtConfig};
use url::Url;

const ALPN_H3: &[u8] = b"h3";

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn server_endpoint(addr: SocketAddr) -> (Arc<quinn::Endpoint>, rustls::pki_types::CertificateDer<'static>) {
    let (cert_der, key_der) = self_signed_for_dev(&["localhost".into()]).expect("self_signed");
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server tls");
    tls.alpn_protocols = vec![ALPN_H3.to_vec()];

    let endpoint = rvoip_uctp::substrate::make_server_endpoint(
        addr,
        Arc::new(tls),
        quinn::TransportConfig::default(),
    )
    .expect("endpoint");
    (Arc::new(endpoint), cert_der)
}

fn client_endpoint() -> Arc<quinn::Endpoint> {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
    Arc::new(
        quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            None,
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .expect("client endpoint"),
    )
}

#[tokio::test]
async fn loopback_auth_handshake_via_wt_adapter() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");

    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_H3])
        .expect("dispatcher");
    let accept_rx = routes.take(ALPN_H3).expect("h3 channel");

    let cfg = UctpWtConfig::new(Arc::clone(&server_ep), accept_rx, bearer_stub());
    let adapter = UctpWtAdapter::new(cfg).await.expect("adapter");
    let _events = adapter.subscribe_events();

    let client_ep = client_endpoint();
    let client_cfg = rvoip_uctp::substrate::dev_client_config_trusting(&cert_der)
        .expect("client cfg");

    let url = Url::parse(&format!("https://localhost:{}/uctp", server_addr.port()))
        .expect("parse url");
    let client = UctpWtClient::connect(
        &client_ep,
        server_addr,
        &url,
        Arc::new(client_cfg),
    )
    .await
    .expect("client connect");

    let mut inbound = client.take_inbound().expect("first take");

    let payload = auth::AuthHello {
        device: auth::Device {
            id: "dev_test".into(),
            kind: "web".into(),
            platform: "browser-shaped".into(),
            sdk_version: "rvoip-wt-test/0.1".into(),
        },
        auth_methods: vec!["bearer".into()],
        capabilities: serde_json::Value::Object(Default::default()),
    };
    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: "env_hello".into(),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(payload).unwrap(),
    signature: None,
    };
    client.send(env).await.expect("send");

    let reply = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("timed out waiting for challenge")
        .expect("inbound channel closed");

    assert_eq!(reply.msg_type, MessageType::AuthChallenge);
}
