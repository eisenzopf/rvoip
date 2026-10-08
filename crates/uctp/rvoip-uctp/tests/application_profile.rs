//! Opt-in command profiles retain the coordinator's authentication boundary.
use async_trait::async_trait;
use rvoip_auth_core::{bearer_stub, AuthenticatedPrincipal, BearerAuthError, BearerValidator};
use rvoip_core::identity::IdentityAssurance;
use rvoip_uctp::{
    application::{ApplicationContext, ApplicationError, ApplicationHandler},
    envelope::UctpEnvelope,
    payloads::auth,
    state::{UctpCoordinator, ENVELOPE_CHANNEL_CAP},
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

fn coordinator(
    handler: Option<Arc<Handler>>,
    bearer: Arc<dyn BearerValidator>,
) -> (
    Arc<UctpCoordinator>,
    mpsc::Sender<UctpEnvelope>,
    mpsc::Receiver<UctpEnvelope>,
) {
    let (input, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (output, out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coordinator = UctpCoordinator::start("websocket", in_rx, output, events, bearer);
    if let Some(handler) = handler {
        coordinator.set_application_handler(handler);
    }
    (coordinator, input, out_rx)
}

async fn receive(output: &mut mpsc::Receiver<UctpEnvelope>) -> UctpEnvelope {
    tokio::time::timeout(Duration::from_secs(3), output.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn authenticate(
    input: &mpsc::Sender<UctpEnvelope>,
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
    let (coordinator, input, mut output) = coordinator(Some(handler.clone()), bearer_stub());
    let caps = authenticate(&input, &mut output).await;
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
    coordinator.shutdown().await;
    assert!(closed.is_cancelled());
}

#[tokio::test]
async fn authentication_and_profile_scope_run_before_application_effects() {
    let handler = Arc::new(Handler::default());
    let (coordinator, input, mut output) =
        coordinator(Some(handler.clone()), Arc::new(SessionOnly));
    input.send(request()).await.unwrap();
    assert_eq!(receive(&mut output).await.payload["code"], 401);
    authenticate(&input, &mut output).await;
    input.send(request()).await.unwrap();
    assert_eq!(receive(&mut output).await.payload["code"], 403);
    assert_eq!(handler.effects.load(Ordering::SeqCst), 0);
    assert_eq!(handler.replays.load(Ordering::SeqCst), 0);
    coordinator.shutdown().await;
}

#[tokio::test]
async fn absent_or_unknown_profiles_do_not_reach_an_application_handler() {
    for installed in [false, true] {
        let handler = Arc::new(Handler::default());
        let (coordinator, input, mut output) =
            coordinator(installed.then(|| handler.clone()), bearer_stub());
        let caps = authenticate(&input, &mut output).await;
        assert_eq!(caps.get("application_profiles").is_some(), installed);
        let mut command = request();
        command.payload["profile"] = json!("uninstalled/control-v1");
        input.send(command).await.unwrap();
        let reply = receive(&mut output).await;
        if installed {
            assert_eq!(reply.payload["code"], 501);
        } else {
            // Stock deployments never take the profile branch: a `profile`
            // field is ordinary payload data on the legacy dispatch path.
            assert_eq!(reply.payload["reason"], "malformed-data-message");
        }
        // A profile-free envelope still follows legacy message dispatch.
        let mut legacy = request();
        legacy.payload.as_object_mut().unwrap().remove("profile");
        input.send(legacy).await.unwrap();
        assert_eq!(receive(&mut output).await.msg_type, MessageType::Error);
        assert_eq!(handler.effects.load(Ordering::SeqCst), 0);
        coordinator.shutdown().await;
    }
}

#[tokio::test]
async fn profile_field_without_installed_handler_keeps_legacy_dispatch() {
    // A stock coordinator (no handler) must treat a string `payload.profile`
    // as plain data: an unauthenticated peer still gets the legacy 401, and
    // an authenticated `message.send` reaches the legacy handler, which
    // answers exactly as it does for the profile-free envelope.
    let (coordinator, input, mut output) = coordinator(None, bearer_stub());
    authenticate(&input, &mut output).await;
    let with_profile = request();
    let mut without_profile = request();
    without_profile
        .payload
        .as_object_mut()
        .unwrap()
        .remove("profile");
    input.send(without_profile).await.unwrap();
    let legacy = receive(&mut output).await;
    input.send(with_profile).await.unwrap();
    let profiled = receive(&mut output).await;
    assert_eq!(legacy.msg_type, MessageType::Error);
    assert_eq!(profiled.payload["code"], legacy.payload["code"]);
    assert_eq!(profiled.payload["reason"], legacy.payload["reason"]);
    assert_ne!(profiled.payload["code"], 501);
    coordinator.shutdown().await;
}

struct NeverCompletes {
    calls: AtomicUsize,
}

#[async_trait]
impl ApplicationHandler for NeverCompletes {
    fn profile(&self) -> &'static str {
        "example/control-v1"
    }

    async fn handle(
        &self,
        _context: ApplicationContext,
        _request: UctpEnvelope,
    ) -> Result<UctpEnvelope, ApplicationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::future::pending().await
    }
}

#[tokio::test]
async fn stalled_application_handler_times_out_without_wedging_the_peer() {
    use rvoip_uctp::state::UctpCoordinatorCaps;

    let (input, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (output, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let caps = UctpCoordinatorCaps {
        application_handler_timeout: Duration::from_millis(200),
        ..Default::default()
    };
    let coordinator = UctpCoordinator::start_full_with_caps(
        "websocket",
        in_rx,
        output,
        events,
        bearer_stub(),
        Arc::new(rvoip_uctp::state::default_v0_descriptor()),
        rvoip_uctp::state::rejecting_handler(),
        caps,
    );
    let handler = Arc::new(NeverCompletes {
        calls: AtomicUsize::new(0),
    });
    coordinator.set_application_handler(handler.clone());
    authenticate(&input, &mut out_rx).await;

    let request = request();
    input.send(request.clone()).await.unwrap();
    let timed_out = receive(&mut out_rx).await;
    assert_eq!(timed_out.msg_type, MessageType::Error);
    assert_eq!(timed_out.payload["code"], 504);
    assert_eq!(timed_out.payload["reason"], "application-handler-timeout");
    assert_eq!(timed_out.in_reply_to.as_deref(), Some(request.id.as_str()));
    assert_eq!(handler.calls.load(Ordering::SeqCst), 1);

    // The peer driver is free again: legacy signaling is still serviced.
    let mut legacy = UctpEnvelope::new(
        MessageType::MessageSend,
        json!({"to":["part_recipient"], "medium":"sms", "body":"fixture"}),
    );
    legacy.connid = Some("conn_profile".into());
    input.send(legacy.clone()).await.unwrap();
    let after = receive(&mut out_rx).await;
    assert_eq!(after.in_reply_to.as_deref(), Some(legacy.id.as_str()));
    coordinator.shutdown().await;
}
