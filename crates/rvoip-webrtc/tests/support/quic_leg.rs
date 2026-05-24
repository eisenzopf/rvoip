//! Real `rvoip-quic` test harness — quinn endpoint, UCTP auth, session.invite dial.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_quic::{UctpQuicAdapter, UctpQuicClient, UctpQuicConfig};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session::SessionInvite};
use rvoip_uctp::substrate::{dev_client_config_trusting, dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::types::MessageType;

pub const ALPN_UCTP: &[u8] = b"uctp/1";

static ENV_ID: AtomicU64 = AtomicU64::new(0);

fn rand_env_id() -> String {
    format!("env_{:016x}", ENV_ID.fetch_add(1, Ordering::Relaxed))
}

pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub fn client_endpoint() -> Arc<quinn::Endpoint> {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("client udp bind");
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

pub fn server_endpoint(
    addr: SocketAddr,
) -> (
    Arc<quinn::Endpoint>,
    rustls::pki_types::CertificateDer<'static>,
) {
    let (cert_der, key_der) = self_signed_for_dev(&["localhost".into()]).expect("self_signed");
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server tls");
    tls.alpn_protocols = vec![ALPN_UCTP.to_vec()];
    let endpoint = rvoip_uctp::substrate::make_server_endpoint(
        addr,
        Arc::new(tls),
        quinn::TransportConfig::default(),
    )
    .expect("server endpoint");
    (Arc::new(endpoint), cert_der)
}

/// QUIC server + `UctpQuicAdapter` for orchestrator registration.
pub struct QuicLegHarness {
    pub adapter: Arc<UctpQuicAdapter>,
    pub server_addr: SocketAddr,
    pub cert_der: rustls::pki_types::CertificateDer<'static>,
    pub client_ep: Arc<quinn::Endpoint>,
}

impl QuicLegHarness {
    pub async fn start(bind: SocketAddr) -> Self {
        let (server_ep, cert_der) = server_endpoint(bind);
        let server_addr = server_ep.local_addr().expect("local_addr");
        let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP]).expect("dispatch");
        let accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 accept channel");
        let cfg = UctpQuicConfig::new(server_ep, accept_rx, bearer_stub());
        let adapter = UctpQuicAdapter::new(cfg).await.expect("quic adapter");
        Self {
            adapter,
            server_addr,
            cert_der,
            client_ep: client_endpoint(),
        }
    }

    /// Auth handshake + `session.invite` → server-side inbound QUIC connection.
    pub async fn dial_invite(
        &self,
        sid: &str,
        participant: &str,
    ) -> Arc<UctpQuicClient> {
        let client_cfg = dev_client_config_trusting(&self.cert_der).expect("client tls");
        let client = UctpQuicClient::connect(
            &self.client_ep,
            self.server_addr,
            "localhost",
            Arc::new(client_cfg),
        )
        .await
        .expect("quic connect");

        let mut inbound = client.take_inbound().expect("take_inbound");

        let hello = UctpEnvelope {
            v: 1,
            msg_type: MessageType::AuthHello,
            id: rand_env_id(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: None,
            payload: serde_json::to_value(auth::AuthHello {
                device: auth::Device {
                    id: "dev_webrtc_bridge".into(),
                    kind: "desktop".into(),
                    platform: "test".into(),
                    sdk_version: "rvoip-webrtc-bridge/0.1".into(),
                },
                auth_methods: vec!["bearer".into()],
                capabilities: serde_json::Value::Object(Default::default()),
            })
            .unwrap(),
        };
        client.send(hello).await.expect("auth.hello");
        let challenge = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
            .await
            .expect("auth.challenge timeout")
            .expect("inbound closed");
        assert_eq!(challenge.msg_type, MessageType::AuthChallenge);

        let response = UctpEnvelope {
            v: 1,
            msg_type: MessageType::AuthResponse,
            id: rand_env_id(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: Some(challenge.id),
            payload: serde_json::to_value(auth::AuthResponse {
                method: "bearer".into(),
                credential: "test-token".into(),
            })
            .unwrap(),
        };
        client.send(response).await.expect("auth.response");
        let session_reply = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
            .await
            .expect("auth.session timeout")
            .expect("inbound closed");
        assert_eq!(session_reply.msg_type, MessageType::AuthSession);

        let invite = UctpEnvelope {
            v: 1,
            msg_type: MessageType::SessionInvite,
            id: rand_env_id(),
            ts: Utc::now(),
            cid: Some(format!("conv_{}", rand_env_id())),
            sid: Some(sid.into()),
            connid: None,
            in_reply_to: None,
            payload: serde_json::to_value(SessionInvite {
                from: participant.into(),
                to: vec!["bridge_peer".into()],
                medium: "voice".into(),
                intent: "synchronous-engagement".into(),
                capabilities_offer: serde_json::Value::Object(Default::default()),
            })
            .unwrap(),
        };
        client.send(invite).await.expect("session.invite");
        client
    }
}
