//! WebSocket transport through the rvoip core: authentication, atomic
//! admission, ordered unpaced media to an ordered-mode Vapi adapter, barge-in,
//! digits, and teardown.

mod common;

use base64::{engine::general_purpose::STANDARD, Engine};
use common::*;
use futures::{SinkExt, StreamExt};
use rvoip_core::{
    adapter::{AdapterEvent, ConnectionAdapter, RejectReason},
    commands::InboundAction,
    config::Config as CoreConfig,
    conversation::ConversationPolicy,
    events::Event,
    ids::{ParticipantId, TenantId},
    session::SessionMedium,
    Orchestrator,
};
use rvoip_webex_byova::{
    ReplyAction, ReplyBoundary, ReplyMark, WebexAuthenticator, WebexByovaAdapter, WebexByovaCall,
    WebexError, WebexInput, WebexJwsValidator, WebexTransportKind, WebexWebSocket,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{io::DuplexStream, sync::mpsc, time::timeout};
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message},
    WebSocketStream,
};

type Socket = WebSocketStream<DuplexStream>;

struct Client {
    socket: Socket,
    seq: u64,
}

impl Client {
    async fn send(&mut self, input: Value) {
        self.seq += 1;
        let message = json!({"type":"VOICE_VA_REQUEST","seq":self.seq,
            "ts":"2026-11-15T12:00:00Z","conversation_id":"webex-conversation-1",
            "payload":{"conversation_id":"webex-conversation-1","customer_org_id":ORG,
            "virtual_agent_id":AGENT,"voice_va_input_type":input}});
        self.socket
            .send(Message::Text(message.to_string().into()))
            .await
            .expect("client send");
    }

    async fn start(&mut self) {
        self.send(
            json!({"event_input":{"event_type":"SESSION_START","parameters":{"caller":"test"}}}),
        )
        .await;
    }

    async fn audio(&mut self, bytes: &[u8]) {
        self.send(
            json!({"audio_input":{"caller_audio_b64":STANDARD.encode(bytes),
            "encoding":"MULAW_FORMAT","sample_rate_hertz":8000}}),
        )
        .await;
    }

    async fn recv(&mut self) -> Value {
        let message = timeout(Duration::from_secs(5), self.socket.next())
            .await
            .expect("Webex response arrived")
            .expect("Webex socket open")
            .expect("Webex socket healthy");
        serde_json::from_str(message.to_text().expect("text")).expect("JSON")
    }
}

fn prompt_audio(message: &Value) -> Vec<u8> {
    message["payload"]["prompts"]
        .as_array()
        .map(|prompts| {
            prompts
                .iter()
                .flat_map(|prompt| {
                    STANDARD
                        .decode(prompt["audio_content_b64"].as_str().unwrap_or_default())
                        .expect("Base64 prompt")
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_final(message: &Value) -> bool {
    message["payload"]["response_type"] == "FINAL"
}

fn has_event(message: &Value, name: &str) -> bool {
    message["payload"]["output_events"]
        .as_array()
        .is_some_and(|events| events.iter().any(|event| event["event_type"] == name))
}

/// Upgrade with an `Authorization` header (or none) on `path`.
async fn upgrade(
    adapter: Arc<WebexByovaAdapter>,
    path: &str,
    token: Option<String>,
) -> (
    tokio::task::JoinHandle<rvoip_webex_byova::Result<WebexWebSocket>>,
    Result<Socket, tokio_tungstenite::tungstenite::Error>,
) {
    let (client, server) = tokio::io::duplex(512 * 1024);
    let task = tokio::spawn(async move { adapter.accept_websocket(server).await });
    let mut request = format!("ws://connector.test{path}")
        .into_client_request()
        .expect("request");
    if let Some(token) = token {
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {token}").parse().expect("header"),
        );
    }
    let socket = tokio_tungstenite::client_async(request, client)
        .await
        .map(|(socket, _)| socket);
    (task, socket)
}

async fn start_conversation(adapter: Arc<WebexByovaAdapter>) -> (WebexByovaCall, Client) {
    let (task, socket) = upgrade(adapter, "/v1/va", Some(token(false))).await;
    let mut client = Client {
        socket: socket.expect("authenticated upgrade"),
        seq: 0,
    };
    client.start().await;
    match task.await.expect("accept task").expect("admitted") {
        WebexWebSocket::Conversation(call) => (call, client),
        WebexWebSocket::AgentsListed => panic!("expected a conversation"),
    }
}

#[tokio::test]
async fn upgrade_rejects_missing_invalid_or_misbound_tokens() {
    let mut cases: Vec<(&str, Option<String>)> = vec![("/v1/va", None)];
    let mut expired = claims(false);
    expired["iat"] = json!(chrono::Utc::now().timestamp() - 7200);
    expired["exp"] = json!(chrono::Utc::now().timestamp() - 3600);
    let mut wrong_nonce = claims(false);
    wrong_nonce["jti"] = json!("another-registration-nonce");
    let mut wrong_org = claims(false);
    wrong_org["com.cisco.user.orgid"] = json!("another-org");
    let mut wrong_issuer = claims(false);
    wrong_issuer["iss"] = json!("https://attacker.test.invalid/idb");
    let mut too_long = claims(false);
    too_long["exp"] = json!(chrono::Utc::now().timestamp() + 2 * 24 * 3600);
    for claims in [expired, wrong_nonce, wrong_org, wrong_issuer, too_long] {
        cases.push(("/v1/va", Some(sign(&claims, KID))));
    }
    cases.push(("/v1/va", Some(sign(&common::claims(false), "unknown-key"))));
    // A gRPC registration's token cannot open a WebSocket conversation.
    cases.push(("/v1/va", Some(token(true))));
    // An unsigned token.
    let unsigned = token(false);
    let mut parts: Vec<&str> = unsigned.split('.').collect();
    parts[2] = "";
    cases.push(("/v1/va", Some(parts.join("."))));
    // A valid token on an unknown path.
    cases.push(("/admin", Some(token(false))));
    for (path, token) in cases {
        let adapter = WebexByovaAdapter::new(config()).expect("adapter");
        let (task, socket) = upgrade(adapter.clone(), path, token).await;
        assert!(socket.is_err(), "upgrade on {path} was accepted");
        assert!(task.await.expect("task").is_err());
        assert_eq!(adapter.connection_count(), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn stale_signing_keys_fail_closed_until_replaced() {
    let validator = WebexJwsValidator::new(jws_config(), jwks()).expect("validator");
    let token = token(false);
    assert!(validator
        .authenticate(&token, WebexTransportKind::WebSocket)
        .is_ok());
    tokio::time::advance(Duration::from_secs(3601)).await;
    assert_eq!(
        validator
            .authenticate(&token, WebexTransportKind::WebSocket)
            .unwrap_err(),
        WebexError::KeysUnavailable
    );
    validator.replace_keys(jwks());
    let principal = validator
        .authenticate(&token, WebexTransportKind::WebSocket)
        .expect("refreshed keys");
    assert_eq!(principal.organization_id, ORG);
    assert!(!format!("{principal:?}").contains(ORG));
}

#[tokio::test]
async fn conversation_is_published_with_its_principal_only_after_session_start() {
    let adapter = WebexByovaAdapter::new(config()).expect("adapter");
    let mut events = adapter.subscribe_events();
    let (task, socket) = upgrade(adapter.clone(), "/v1/va", Some(token(false))).await;
    let mut client = Client {
        socket: socket.expect("upgrade"),
        seq: 0,
    };
    // Authenticated but not started: nothing is published.
    assert!(timeout(Duration::from_millis(50), events.recv())
        .await
        .is_err());
    client.start().await;
    let WebexWebSocket::Conversation(mut call) = task.await.unwrap().unwrap() else {
        panic!("expected a conversation");
    };
    assert_eq!(call.organization_id(), ORG);
    assert_eq!(call.start_parameters()["caller"], "test");
    let Some(AdapterEvent::InboundConnection { connection }) = events.recv().await else {
        panic!("expected the inbound connection first");
    };
    assert_eq!(&connection.id, call.connection_id());
    assert_eq!(
        connection.transport,
        rvoip_core::connection::Transport::WebexByova
    );
    let Some(AdapterEvent::PrincipalAuthenticated { principal, .. }) = events.recv().await else {
        panic!("expected the principal with the connection");
    };
    assert_eq!(principal.tenant.as_deref(), Some(ORG));
    // No reply before core accepts.
    assert!(timeout(Duration::from_millis(50), client.socket.next())
        .await
        .is_err());
    adapter.accept(call.connection_id().clone()).await.unwrap();
    assert!(matches!(call.next_input().await, Some(WebexInput::Started)));
    // A second session with the same Webex conversation is refused.
    let (task, socket) = upgrade(adapter.clone(), "/v1/va", Some(token(false))).await;
    let mut duplicate = Client {
        socket: socket.expect("upgrade"),
        seq: 0,
    };
    duplicate.start().await;
    assert_eq!(
        task.await.unwrap().unwrap_err(),
        WebexError::DuplicateConversation
    );
    call.close().await;
    assert!(!adapter.is_connection_live(call.connection_id()));
}

#[tokio::test]
async fn admission_gate_rejection_closes_the_conversation() {
    let core = Orchestrator::new(CoreConfig::default());
    let mut admissions = core
        .install_inbound_admission_gate(4, Duration::from_secs(5))
        .expect("gate");
    let adapter = WebexByovaAdapter::new(config()).expect("adapter");
    core.register(adapter.clone()).expect("register");
    let (call, mut client) = start_conversation(adapter.clone()).await;
    let ticket = timeout(Duration::from_secs(2), admissions.recv())
        .await
        .expect("admission ticket")
        .expect("gate open");
    assert_eq!(ticket.connection_id(), call.connection_id());
    assert_eq!(
        ticket
            .authenticated_principal()
            .expect("principal")
            .tenant
            .as_deref(),
        Some(ORG)
    );
    ticket
        .reject(RejectReason::Forbidden)
        .await
        .expect("reject");
    timeout(Duration::from_secs(3), call.closed())
        .await
        .expect("rejected conversation closed");
    assert!(!adapter.is_connection_live(call.connection_id()));
    // Nothing but a close reaches Webex.
    let next = timeout(Duration::from_secs(2), client.socket.next())
        .await
        .expect("socket settled");
    assert!(matches!(
        next,
        None | Some(Ok(Message::Close(_))) | Some(Err(_))
    ));
}

#[tokio::test]
async fn list_agents_is_authenticated_and_tenant_bound() {
    let adapter = WebexByovaAdapter::new(config()).expect("adapter");
    for (org, expected) in [(ORG, Some(AGENT)), ("another-org", None)] {
        let (task, socket) =
            upgrade(adapter.clone(), "/v1/listVirtualAgents", Some(token(false))).await;
        let mut socket = socket.expect("upgrade");
        socket
            .send(Message::Text(
                json!({"customer_org_id":org}).to_string().into(),
            ))
            .await
            .unwrap();
        match expected {
            Some(agent) => {
                let reply: Value =
                    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                assert_eq!(reply["virtual_agents"][0]["id"], agent);
                assert!(matches!(
                    task.await.unwrap().unwrap(),
                    WebexWebSocket::AgentsListed
                ));
            }
            None => assert_eq!(task.await.unwrap().unwrap_err(), WebexError::Unauthorized),
        }
    }
}

#[tokio::test]
async fn local_end_sends_session_end_and_releases_the_slot() {
    let mut cfg = config();
    cfg.max_connections = 1;
    let adapter = WebexByovaAdapter::new(cfg).expect("adapter");
    let _events = adapter.subscribe_events();
    let (mut call, mut client) = start_conversation(adapter.clone()).await;
    adapter.accept(call.connection_id().clone()).await.unwrap();
    assert!(matches!(call.next_input().await, Some(WebexInput::Started)));
    call.end_session().await.expect("end session");
    let end = client.recv().await;
    assert!(is_final(&end) && has_event(&end, "SESSION_END"));
    timeout(Duration::from_secs(2), call.closed())
        .await
        .unwrap();
    // The slot is free again.
    let (again, _client) = start_conversation(adapter.clone()).await;
    again.close().await;
}

#[tokio::test]
async fn binary_frames_and_sequence_regressions_fail_closed() {
    for binary in [true, false] {
        let adapter = WebexByovaAdapter::new(config()).expect("adapter");
        let _events = adapter.subscribe_events();
        let (mut call, mut client) = start_conversation(adapter.clone()).await;
        adapter.accept(call.connection_id().clone()).await.unwrap();
        assert!(matches!(call.next_input().await, Some(WebexInput::Started)));
        if binary {
            client
                .socket
                .send(Message::Binary(vec![0xff; 160].into()))
                .await
                .unwrap();
        } else {
            client.seq = 0; // Replays sequence number 1.
            client.audio(&[0xff; 320]).await;
            let error = client.recv().await;
            assert_eq!(error["type"], "ERROR");
        }
        timeout(Duration::from_secs(3), call.closed())
            .await
            .unwrap();
        assert_eq!(adapter.connection_count(), 0);
    }
}

/// A scripted Vapi WebSocket: plays what the test sends it and reports the
/// caller audio and control messages rvoip writes.
async fn mock_vapi() -> (
    url::Url,
    mpsc::Sender<Message>,
    mpsc::Receiver<Message>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (script, mut script_rx) = mpsc::channel::<Message>(64);
    let (observed_tx, observed) = mpsc::channel::<Message>(256);
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        loop {
            tokio::select! {
                Some(message) = script_rx.recv() => {
                    if ws.send(message).await.is_err() { break; }
                }
                incoming = ws.next() => match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(message)) => {
                        let end = message.to_text().is_ok_and(|text| text.contains("end-call"));
                        let _ = observed_tx.send(message).await;
                        if end {
                            let _ = ws.close(None).await;
                            break;
                        }
                    }
                },
            }
        }
    });
    (
        url::Url::parse(&format!("ws://{address}/audio")).unwrap(),
        script,
        observed,
        task,
    )
}

fn vapi_event(value: Value) -> Message {
    Message::Text(value.to_string().into())
}

/// Map Vapi's ordered events to Webex reply marks: the assistant's
/// speech-stop completes a reply and a user interruption interrupts it.
fn reply_marks(
    events: mpsc::Receiver<rvoip_vapi::VapiOrderedEvent>,
) -> impl futures::Stream<Item = ReplyMark> + Send {
    futures::stream::unfold(events, |mut events| async move {
        events.recv().await.map(|event| (event, events))
    })
    .filter_map(|ordered| {
        let action = match &ordered.event {
            rvoip_vapi::VapiEvent::SpeechUpdate {
                role: Some(role),
                status,
                ..
            } if role == "assistant" && status == "stopped" => Some(ReplyAction::Complete),
            rvoip_vapi::VapiEvent::Unknown(value) if value["type"] == "user-interrupted" => {
                Some(ReplyAction::Interrupt)
            }
            _ => None,
        };
        futures::future::ready(action.map(|action| ReplyMark {
            position: ordered.audio_offset,
            action,
        }))
    })
}

/// Collect Webex output until a FINAL, returning (messages, audio bytes).
async fn until_final(client: &mut Client) -> (Vec<Value>, Vec<u8>) {
    let mut messages = Vec::new();
    let mut audio = Vec::new();
    loop {
        let message = client.recv().await;
        audio.extend(prompt_audio(&message));
        let done = is_final(&message);
        messages.push(message);
        if done {
            return (messages, audio);
        }
    }
}

#[tokio::test]
async fn orchestrator_bridges_webex_to_ordered_vapi_in_wire_order() {
    use rvoip_vapi::{
        VapiAdapter, VapiApiKey, VapiAssistant, VapiAudioFormat, VapiCallOptions, VapiConfig,
        VapiEvent, VapiExistingCall, VapiMediaDelivery, VapiPeerFailurePolicy,
    };
    let core = Orchestrator::new(CoreConfig::default());
    let adapter = WebexByovaAdapter::new(config()).expect("adapter");
    core.register(adapter.clone()).expect("register Webex");
    let mut core_events = core.subscribe_events();
    let conversation = core
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            Default::default(),
        )
        .await
        .unwrap();
    let session = core
        .start_session(conversation.clone(), SessionMedium::Voice, vec![])
        .await
        .unwrap();

    let (mut call, mut client) = start_conversation(adapter.clone()).await;
    timeout(Duration::from_secs(2), async {
        loop {
            if matches!(core_events.recv().await.unwrap(),
                Event::ConnectionInbound { connection_id, .. } if &connection_id == call.connection_id())
            {
                break;
            }
        }
    })
    .await
    .expect("core published the Webex connection");
    core.route_inbound_connection(
        call.connection_id().clone(),
        InboundAction::Accept {
            session_id: session,
            participant_id: ParticipantId::new(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(call.next_input().await, Some(WebexInput::Started)));

    let (vapi_url, script, mut observed, vapi_task) = mock_vapi().await;
    let mut vapi_config = VapiConfig::existing_calls_only().with_loopback_test_transport();
    vapi_config.graceful_shutdown_timeout = Duration::from_millis(200);
    vapi_config.heartbeat_interval = Duration::from_secs(60);
    let vapi = VapiAdapter::new(vapi_config).unwrap();
    let options = VapiCallOptions::new(VapiAssistant::saved("test-assistant"))
        .with_audio_format(VapiAudioFormat::MuLaw8Khz)
        .with_media_delivery(VapiMediaDelivery::Ordered)
        .with_peer_failure_policy(VapiPeerFailurePolicy::LeaveCallerConnected);
    let existing = VapiExistingCall::new(options, "verified-test-call".into(), vapi_url)
        .unwrap()
        .with_api_key(VapiApiKey::new("test-vapi-key").unwrap());
    let agent = vapi
        .attach_existing_agent(&core, call.connection_id().clone(), existing)
        .await
        .expect("attach ordered Vapi");
    let mut ordered = agent.take_ordered_events().expect("ordered events");

    // Greeting: a one-second burst, then the assistant's speech-stop. The
    // event's audio position is exactly the burst, and FINAL follows every
    // byte of it.
    let started = tokio::time::Instant::now();
    script
        .send(Message::Binary(vec![0x11; 4_000].into()))
        .await
        .unwrap();
    script
        .send(Message::Binary(vec![0x22; 4_000].into()))
        .await
        .unwrap();
    script
        .send(vapi_event(
            json!({"type":"speech-update","role":"assistant","status":"stopped"}),
        ))
        .await
        .unwrap();
    let stop = timeout(Duration::from_secs(3), ordered.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stop.audio_offset, 8_000);
    assert!(matches!(stop.event, VapiEvent::SpeechUpdate { .. }));
    call.complete_reply(ReplyBoundary::At(stop.audio_offset))
        .await
        .expect("greeting completes");
    let (messages, audio) = until_final(&mut client).await;
    assert_eq!(audio.len(), 8_000);
    assert!(audio[..4_000].iter().all(|byte| *byte == 0x11));
    assert!(audio[4_000..].iter().all(|byte| *byte == 0x22));
    assert_eq!(
        messages.iter().filter(|message| is_final(message)).count(),
        1
    );
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "one second of audio was paced"
    );
    assert_eq!(call.agent_audio_position(), 8_000);

    // Caller audio reaches Vapi unchanged and unpaced.
    let caller: Vec<u8> = (0..640u32).map(|n| (n % 251) as u8).collect();
    client.audio(&caller).await;
    let mut written = Vec::new();
    while written.len() < caller.len() {
        match timeout(Duration::from_secs(3), observed.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Message::Binary(bytes) => written.extend_from_slice(&bytes),
            _ => {}
        }
    }
    assert_eq!(written, caller);

    // From here the session follows Vapi's ordered events itself, so each
    // boundary lands on its exact byte even though all of this audio is
    // already queued when the events are read.
    call.follow_reply_marks(reply_marks(ordered))
        .await
        .expect("follow marks");

    // Barge-in: the interrupted reply ends exactly where Vapi reported the
    // interruption, no stale byte follows START_OF_INPUT, and the next reply
    // carries only the new audio.
    script
        .send(Message::Binary(vec![0x33; 8_000].into()))
        .await
        .unwrap();
    script
        .send(vapi_event(json!({"type":"user-interrupted"})))
        .await
        .unwrap();
    script
        .send(Message::Binary(vec![0x44; 1_600].into()))
        .await
        .unwrap();
    script
        .send(vapi_event(
            json!({"type":"speech-update","role":"assistant","status":"stopped"}),
        ))
        .await
        .unwrap();
    let mut before = Vec::new();
    let mut after = None::<Vec<u8>>;
    let mut finals_after = 0;
    while finals_after < 2 {
        let message = client.recv().await;
        let audio = prompt_audio(&message);
        if has_event(&message, "START_OF_INPUT") {
            assert!(audio.is_empty());
            after = Some(Vec::new());
            continue;
        }
        match after.as_mut() {
            Some(after) => {
                after.extend(audio);
                if is_final(&message) {
                    finals_after += 1;
                }
            }
            None => {
                assert!(!is_final(&message), "the interrupted reply finished early");
                before.extend(audio);
            }
        }
    }
    // Everything before the boundary is the interrupted reply's audio (its
    // unsent 320-byte tail is discarded with it).
    assert!(before.iter().all(|byte| *byte == 0x33));
    assert_eq!(before.len(), 8_000 - 320);
    assert_eq!(after.expect("START_OF_INPUT sent"), vec![0x44; 1_600]);
    assert_eq!(call.agent_audio_position(), 17_600);

    // Digits arrive in order as one input.
    client
        .send(json!({"dtmf_input":{"dtmf_events":["DTMF_DIGIT_ONE","DTMF_DIGIT_TWO","DTMF_DIGIT_POUND"]}}))
        .await;
    assert!(matches!(call.next_input().await, Some(WebexInput::Dtmf(digits)) if digits == "12#"));

    // Webex ends the session: the call closes, core tears the bridge down,
    // and the Vapi leg is ended with end-call.
    client
        .send(json!({"event_input":{"event_type":"SESSION_END"}}))
        .await;
    assert!(matches!(call.next_input().await, Some(WebexInput::Ended)));
    timeout(Duration::from_secs(3), call.closed())
        .await
        .unwrap();
    timeout(Duration::from_secs(5), agent.wait_shared())
        .await
        .expect("Vapi leg ended");
    let mut ended = false;
    while let Ok(Some(message)) = timeout(Duration::from_secs(2), observed.recv()).await {
        if message
            .to_text()
            .is_ok_and(|text| text.contains("end-call"))
        {
            ended = true;
            break;
        }
    }
    assert!(ended, "Vapi call was not ended");
    assert!(!adapter.is_connection_live(call.connection_id()));
    assert_eq!(adapter.connection_count(), 0);
    let _ = timeout(Duration::from_secs(2), vapi_task).await;
    core.close_conversation(conversation, true).await.unwrap();
}
