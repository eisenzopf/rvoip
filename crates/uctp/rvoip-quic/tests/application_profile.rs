//! Application profiles over a real TLS/QUIC connection retain UCTP authentication,
//! correlation, replay and legacy dispatch behavior.
use async_trait::async_trait;
use rvoip_auth_core::{bearer_stub, AuthenticatedPrincipal, BearerAuthError, BearerValidator};
use rvoip_core::identity::IdentityAssurance;
use rvoip_quic::{UctpQuicAdapter, UctpQuicClient, UctpQuicConfig};
use rvoip_uctp::{
    application::{ApplicationContext, ApplicationError, ApplicationHandler},
    envelope::UctpEnvelope,
    payloads::auth,
    types::MessageType,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Handler {
    effects: AtomicUsize,
    replays: AtomicUsize,
    stored: Mutex<Option<(UctpEnvelope, UctpEnvelope)>>,
    closed: Mutex<Option<CancellationToken>>,
}

#[async_trait]
impl ApplicationHandler for Handler {
    fn profile(&self) -> &'static str {
        "example/control-v1"
    }

    async fn handle(
        &self,
        context: ApplicationContext,
        request: UctpEnvelope,
    ) -> Result<UctpEnvelope, ApplicationError> {
        assert!(context.principal.has_scope(self.required_scope()));
        self.effects.fetch_add(1, Ordering::SeqCst);
        *self.closed.lock().unwrap() = Some(context.closed);
        let reply = UctpEnvelope::new(MessageType::Ack, json!({"accepted":true}));
        *self.stored.lock().unwrap() = Some((request, reply.clone()));
        Ok(reply)
    }

    async fn replay(
        &self,
        _context: ApplicationContext,
        request: UctpEnvelope,
    ) -> Result<UctpEnvelope, ApplicationError> {
        self.replays.fetch_add(1, Ordering::SeqCst);
        let stored = self.stored.lock().unwrap();
        let (old, reply) = stored
            .as_ref()
            .ok_or_else(|| ApplicationError::new(409, "no-stored-outcome"))?;
        if old.payload != request.payload
            || old.cid != request.cid
            || old.sid != request.sid
            || old.connid != request.connid
        {
            return Err(ApplicationError::new(409, "request-content-changed"));
        }
        Ok(reply.clone())
    }
}

struct SessionOnly;
#[async_trait]
impl BearerValidator for SessionOnly {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        Ok(self.validate_principal(token).await?.assurance)
    }
    async fn validate_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        let mut principal = bearer_stub().validate_principal(token).await?;
        principal.scopes = vec!["uctp:session".into()];
        Ok(principal)
    }
}

struct Fixture {
    server: Arc<quinn::Endpoint>,
    client_endpoint: Arc<quinn::Endpoint>,
    adapter: Arc<UctpQuicAdapter>,
    client: Arc<UctpQuicClient>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.adapter.begin_drain();
        self.server.close(0u32.into(), b"test complete");
        self.client_endpoint.close(0u32.into(), b"test complete");
    }
}

async fn connect(
    handler: Option<Arc<Handler>>,
    bearer: Arc<dyn BearerValidator>,
) -> (Fixture, mpsc::Receiver<UctpEnvelope>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert, key) = rvoip_uctp::substrate::self_signed_for_dev(&["localhost".into()]).unwrap();
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key)
        .unwrap();
    tls.alpn_protocols = vec![rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES.to_vec()];
    let server = Arc::new(
        rvoip_uctp::substrate::make_server_endpoint(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(tls),
            quinn::TransportConfig::default(),
        )
        .unwrap(),
    );
    let mut routes = rvoip_uctp::substrate::dispatch_by_alpn(
        server.clone(),
        &[rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES],
    )
    .unwrap();
    let mut config = UctpQuicConfig::new(
        server.clone(),
        routes.take(rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES).unwrap(),
        bearer,
    );
    if let Some(handler) = handler {
        config = config.with_application_handler(handler);
    }
    let adapter = UctpQuicAdapter::new(config).await.unwrap();
    let client_endpoint =
        Arc::new(quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap());
    let client_tls = rvoip_uctp::substrate::dev_client_config_trusting(&cert).unwrap();
    let client = UctpQuicClient::connect(
        &client_endpoint,
        server.local_addr().unwrap(),
        "localhost",
        Arc::new(client_tls),
    )
    .await
    .unwrap();
    let inbound = client.take_inbound().unwrap();
    (
        Fixture {
            server,
            client_endpoint,
            adapter,
            client,
        },
        inbound,
    )
}

async fn receive(output: &mut mpsc::Receiver<UctpEnvelope>) -> UctpEnvelope {
    tokio::time::timeout(Duration::from_secs(3), output.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn authenticate(
    input: &UctpQuicClient,
    output: &mut mpsc::Receiver<UctpEnvelope>,
) -> serde_json::Value {
    input
        .send(UctpEnvelope::new(
            MessageType::AuthHello,
            serde_json::to_value(auth::AuthHello {
                device: auth::Device {
                    id: "dev_profile".into(),
                    kind: "desktop".into(),
                    platform: "test".into(),
                    sdk_version: "test/1".into(),
                },
                auth_methods: vec!["bearer".into()],
                capabilities: json!({}),
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    let challenge = receive(output).await;
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
    let capabilities = challenge.payload["server_capabilities"].clone();
    let mut response = UctpEnvelope::new(
        MessageType::AuthResponse,
        serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "test-token".into(),
            actor_token: None,
        })
        .unwrap(),
    );
    response.in_reply_to = Some(challenge.id);
    input.send(response).await.unwrap();
    assert_eq!(receive(output).await.msg_type, MessageType::AuthSession);
    capabilities
}

fn request() -> UctpEnvelope {
    let mut request = UctpEnvelope::new(
        MessageType::MessageSend,
        json!({
            "profile":"example/control-v1", "to":["part_recipient"], "medium":"sms", "body":"fixture"
        }),
    );
    request.cid = Some("conv_profile".into());
    request.sid = Some("sess_profile".into());
    request.connid = Some("conn_profile".into());
    request
}

#[tokio::test]
async fn profile_preserves_context_correlates_replies_and_replays_without_effects() {
    let handler = Arc::new(Handler::default());
    let (fixture, mut output) = connect(Some(handler.clone()), bearer_stub()).await;
    let input = &fixture.client;
    let caps = authenticate(input, &mut output).await;
    assert_eq!(caps["application_profiles"], json!(["example/control-v1"]));
    let request = request();
    input.send(request.clone()).await.unwrap();
    let first = receive(&mut output).await;
    assert_eq!(first.msg_type, MessageType::Ack);
    assert_eq!(first.in_reply_to.as_deref(), Some(request.id.as_str()));
    assert_eq!(
        (&first.cid, &first.sid, &first.connid),
        (&request.cid, &request.sid, &request.connid)
    );
    assert_eq!(
        handler.stored.lock().unwrap().as_ref().unwrap().0.payload,
        request.payload
    );

    input.send(request.clone()).await.unwrap();
    assert_eq!(receive(&mut output).await.payload, first.payload);
    let mut changed = request;
    changed.payload["body"] = json!("different effect");
    input.send(changed).await.unwrap();
    assert_eq!(receive(&mut output).await.payload["code"], 409);
    assert_eq!(handler.effects.load(Ordering::SeqCst), 1);
    assert_eq!(handler.replays.load(Ordering::SeqCst), 2);

    let closed = handler.closed.lock().unwrap().clone().unwrap();
    fixture.client.connection.close(0u32.into(), b"peer done");
    tokio::time::timeout(Duration::from_secs(3), closed.cancelled())
        .await
        .unwrap();
}

#[tokio::test]
async fn authentication_and_profile_scope_run_before_application_effects() {
    let handler = Arc::new(Handler::default());
    let (fixture, mut output) = connect(Some(handler.clone()), Arc::new(SessionOnly)).await;
    let input = &fixture.client;
    input.send(request()).await.unwrap();
    assert_eq!(receive(&mut output).await.payload["code"], 401);
    authenticate(input, &mut output).await;
    input.send(request()).await.unwrap();
    assert_eq!(receive(&mut output).await.payload["code"], 403);
    assert_eq!(handler.effects.load(Ordering::SeqCst), 0);
    assert_eq!(handler.replays.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn absent_or_unknown_profiles_do_not_reach_an_application_handler() {
    for installed in [false, true] {
        let handler = Arc::new(Handler::default());
        let (fixture, mut output) =
            connect(installed.then(|| handler.clone()), bearer_stub()).await;
        let input = &fixture.client;
        let caps = authenticate(input, &mut output).await;
        assert_eq!(caps.get("application_profiles").is_some(), installed);
        let mut command = request();
        command.payload["profile"] = json!("uninstalled/control-v1");
        input.send(command).await.unwrap();
        assert_eq!(receive(&mut output).await.payload["code"], 501);
        // A profile-free envelope still follows legacy message dispatch.
        let mut legacy = request();
        legacy.payload.as_object_mut().unwrap().remove("profile");
        input.send(legacy).await.unwrap();
        assert_eq!(receive(&mut output).await.msg_type, MessageType::Error);
        assert_eq!(handler.effects.load(Ordering::SeqCst), 0);
    }
}
