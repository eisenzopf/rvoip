use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rvoip_amazon_a2a::{
    protocol::{EVENT_TYPE, EXTENSION},
    *,
};
use rvoip_core::{
    adapter::{AdapterEvent, ConnectionAdapter},
    ids::{ParticipantId, SessionId},
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{io::DuplexStream, time::timeout};
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message},
    WebSocketStream,
};
const KEY: &str = "test-key-not-a-real-credential";
type Socket = WebSocketStream<DuplexStream>;
fn config() -> AmazonA2aConfig {
    AmazonA2aConfig::new(ApiKey::new(KEY).unwrap())
}
fn binding() -> CallBinding {
    CallBinding {
        session_id: SessionId::new(),
        participant_id: ParticipantId::new(),
        instance_arn: "arn:test:instance".into(),
    }
}
fn request(event: Option<&str>, data: Value) -> Value {
    let mut metadata = json!({});
    if let Some(event) = event {
        metadata[EVENT_TYPE] = json!(event);
    }
    json!({"jsonrpc":"2.0","id":"request-1","method":"SendMessage","params":{"message":{"messageId":"msg-1","contextId":"ctx-1","role":"ROLE_USER","metadata":metadata,"parts":[data]}}})
}
fn init(audio: bool) -> Value {
    let mut data = json!({"instanceArn":"arn:test:instance","contactArn":"arn:test:contact","supportedFinishTypes":["COMPLETE","ESCALATE"],"subscribeToTracingEvents":true,"history":[]});
    if audio {
        data["audioInputConfiguration"] = json!(AudioConfiguration::pcm(8000));
        data["audioOutputConfiguration"] = json!(AudioConfiguration::pcm(24000));
    }
    request(
        Some("INIT_SESSION"),
        json!({"data":{"initSession":data},"mediaType":"application/json"}),
    )
}
async fn send(socket: &mut Socket, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}
async fn recv(socket: &mut Socket) -> Value {
    let message = timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}
async fn handshake(adapter: Arc<AmazonA2aAdapter>, audio: bool) -> (AmazonA2aCall, Socket) {
    handshake_bound(adapter, audio, binding()).await
}
async fn handshake_bound(
    adapter: Arc<AmazonA2aAdapter>,
    audio: bool,
    bind: CallBinding,
) -> (AmazonA2aCall, Socket) {
    let (client, server) = tokio::io::duplex(256 * 1024);
    let task = tokio::spawn(async move { adapter.accept_websocket(server, bind).await });
    let mut upgrade = "ws://localhost/a2a".into_client_request().unwrap();
    upgrade
        .headers_mut()
        .insert("authorization", format!("Bearer {KEY}").parse().unwrap());
    upgrade
        .headers_mut()
        .insert("a2a-extensions", EXTENSION.parse().unwrap());
    upgrade.headers_mut().insert(
        "traceparent",
        "00-550e8400e29b41d4a716446655440000-6ba7b8109dad11d1-01"
            .parse()
            .unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::client_async(upgrade, client)
        .await
        .unwrap();
    assert_eq!(response.headers()["a2a-extensions"], EXTENSION);
    send(&mut socket, init(audio)).await;
    (task.await.unwrap().unwrap(), socket)
}
async fn start_with(
    cfg: AmazonA2aConfig,
    audio: bool,
) -> (Arc<AmazonA2aAdapter>, AmazonA2aCall, Socket) {
    let adapter = AmazonA2aAdapter::new(cfg).unwrap();
    let mut events = adapter.subscribe_events();
    let (mut call, mut socket) = handshake(adapter.clone(), audio).await;
    assert!(matches!(
        events.recv().await,
        Some(AdapterEvent::InboundConnection { .. })
    ));
    adapter.accept(call.connection_id().clone()).await.unwrap();
    let response = recv(&mut socket).await;
    assert_eq!(response["id"], "request-1");
    if audio {
        assert_eq!(
            response["result"]["message"]["parts"][0]["data"]["initSessionResponse"]
                ["audioInputConfiguration"]["sampleRateHertz"],
            16000
        );
    }
    send(
        &mut socket,
        request(
            Some("CHANNEL_STATE"),
            json!({"data":{"channelState":{"state":"READY"}}}),
        ),
    )
    .await;
    assert!(matches!(call.next_input().await, Some(InputEvent::Ready)));
    tokio::spawn(async move { while events.recv().await.is_some() {} });
    (adapter, call, socket)
}
async fn start(audio: bool) -> (Arc<AmazonA2aAdapter>, AmazonA2aCall, Socket) {
    start_with(config(), audio).await
}
fn trace() -> TurnTrace {
    TurnTrace::new("test-agent", "customer input", "agent output", 1_000, 2_000).unwrap()
}

#[tokio::test]
async fn roundtrip_pcm_trace_and_finish_are_ordered_and_correlated() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let (adapter, call, mut socket) = start(true).await;
    let stream = adapter
        .streams(call.connection_id().clone())
        .await
        .unwrap()
        .remove(0);
    let mut audio = stream.try_frames_in().unwrap();
    assert!(stream.try_frames_in().is_err());
    assert!(!stream.has_quality_measurement());
    let pcm = vec![1u8; 640];
    send(
        &mut socket,
        request(
            None,
            json!({"raw":STANDARD.encode(&pcm),"mediaType":"audio/lpcm"}),
        ),
    )
    .await;
    assert_eq!(audio.recv().await.unwrap().payload.as_ref(), pcm);
    call.send_audio(Bytes::from(pcm)).await.unwrap();
    let first = recv(&mut socket).await;
    assert_eq!(first["result"]["artifactUpdate"]["append"], false);
    call.complete_turn(trace()).await.unwrap();
    let last = recv(&mut socket).await;
    let tracing = recv(&mut socket).await;
    let terminal = recv(&mut socket).await;
    assert_eq!(last["result"]["artifactUpdate"]["lastChunk"], true);
    let task = &first["result"]["artifactUpdate"]["taskId"];
    assert_eq!(task, &tracing["result"]["artifactUpdate"]["taskId"]);
    assert_eq!(task, &terminal["result"]["statusUpdate"]["taskId"]);
    let span = &tracing["result"]["artifactUpdate"]["artifact"]["parts"][0]["data"]["tracingSpan"]
        ["otlp"]["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert_eq!(span["traceId"], "550e8400e29b41d4a716446655440000");
    assert_eq!(span["parentSpanId"], "6ba7b8109dad11d1");
    assert_eq!(span["events"].as_array().unwrap().len(), 2);
    call.finish(FinishType::Complete, "resolved").await.unwrap();
    assert_eq!(
        recv(&mut socket).await["result"]["message"]["metadata"][EVENT_TYPE],
        "FINISH"
    );
    timeout(Duration::from_secs(2), call.closed())
        .await
        .unwrap();
    assert!(!adapter.is_connection_live(call.connection_id()));
    assert_eq!(
        call.send_audio(Bytes::from_static(&[0, 0])).await,
        Err(A2aError::Closed)
    );
}
#[tokio::test]
async fn output_is_gated_on_ready_and_initialization_on_acceptance() {
    let adapter = AmazonA2aAdapter::new(config()).unwrap();
    let _events = adapter.subscribe_events();
    let (call, mut socket) = handshake(adapter.clone(), true).await;
    assert!(timeout(Duration::from_millis(20), socket.next())
        .await
        .is_err());
    adapter.accept(call.connection_id().clone()).await.unwrap();
    recv(&mut socket).await;
    assert_eq!(
        call.send_audio(Bytes::from_static(&[0, 0])).await,
        Err(A2aError::State)
    );
    call.close().await;
}
#[tokio::test]
async fn peer_interruption_does_not_close_invalidated_artifact() {
    let (_, mut call, mut socket) = start(true).await;
    call.send_audio(Bytes::from(vec![0; 640])).await.unwrap();
    recv(&mut socket).await;
    send(
        &mut socket,
        request(
            Some("INTERRUPTION"),
            json!({"data":{"interruption":{"type":"USER_AUDIO_INPUT"}}}),
        ),
    )
    .await;
    assert!(matches!(
        call.next_input().await,
        Some(InputEvent::Interrupted(_))
    ));
    assert_eq!(
        call.send_audio(Bytes::from(vec![0; 640])).await,
        Err(A2aError::State)
    );
    call.complete_turn(trace()).await.unwrap();
    assert_eq!(
        recv(&mut socket).await["result"]["artifactUpdate"]["artifact"]["metadata"][EVENT_TYPE],
        "TRACING_SPAN"
    );
    recv(&mut socket).await;
    call.send_audio(Bytes::from(vec![0; 640])).await.unwrap();
    assert_eq!(
        recv(&mut socket).await["result"]["artifactUpdate"]["append"],
        false
    );
    call.close().await;
}
#[tokio::test]
async fn local_audio_interruption_flushes_then_closes_artifact_and_traces() {
    let (_, call, mut socket) = start(true).await;
    call.send_audio(Bytes::from(vec![0; 640])).await.unwrap();
    recv(&mut socket).await;
    call.interrupt(InterruptionType::UserAudioInput)
        .await
        .unwrap();
    assert_eq!(
        recv(&mut socket).await["result"]["message"]["metadata"][EVENT_TYPE],
        "INTERRUPTION"
    );
    assert_eq!(
        recv(&mut socket).await["result"]["artifactUpdate"]["lastChunk"],
        true
    );
    call.complete_turn(trace()).await.unwrap();
    recv(&mut socket).await;
    recv(&mut socket).await;
    call.close().await;
}
#[tokio::test]
async fn text_silence_dtmf_and_diagnostic_inputs_remain_distinct() {
    let (_, mut call, mut socket) = start(false).await;
    send(
        &mut socket,
        request(
            None,
            json!({"text":"<EMPTY_USER_INPUT>","mediaType":"text/plain"}),
        ),
    )
    .await;
    assert!(matches!(call.next_input().await,Some(InputEvent::Text(s)) if s=="<EMPTY_USER_INPUT>"));
    send(&mut socket,request(Some("DTMF_INPUT_COMPLETE"),json!({"data":{"dtmfInputComplete":{"inputString":"123","terminationReason":"TIMEOUT"}}}))).await;
    assert!(matches!(call.next_input().await,Some(InputEvent::Dtmf{digits,..}) if digits=="123"));
    send(
        &mut socket,
        request(Some("ERROR"), json!({"data":{"error":{"errorCode":400}}})),
    )
    .await;
    assert!(matches!(
        call.next_input().await,
        Some(InputEvent::Diagnostic(_))
    ));
    call.send_text("response").await.unwrap();
    recv(&mut socket).await;
    assert_eq!(
        call.finish(FinishType::Complete, "premature").await,
        Err(A2aError::State)
    );
    call.complete_turn(trace()).await.unwrap();
    recv(&mut socket).await;
    recv(&mut socket).await;
    recv(&mut socket).await;
    call.close().await;
}
#[tokio::test]
async fn wrong_context_gets_correlated_error_and_cleanup() {
    let (adapter, call, mut socket) = start(false).await;
    let mut value = request(None, json!({"text":"private"}));
    value["params"]["message"]["contextId"] = json!("other");
    send(&mut socket, value).await;
    let error = recv(&mut socket).await;
    assert_eq!(error["id"], "request-1");
    assert_eq!(error["error"]["code"], -32600);
    call.closed().await;
    assert!(!adapter.is_connection_live(call.connection_id()));
}
#[tokio::test]
async fn input_overflow_fails_closed_and_releases_slot() {
    let mut cfg = config();
    cfg.queue_capacity = 2;
    cfg.max_connections = 1;
    let (adapter, call, mut socket) = start_with(cfg, false).await;
    for _ in 0..3 {
        send(&mut socket, request(None, json!({"text":"x"}))).await;
    }
    assert!(recv(&mut socket).await.get("error").is_some());
    call.closed().await;
    let (reconnected, mut ws) = handshake(adapter.clone(), false).await;
    adapter
        .accept(reconnected.connection_id().clone())
        .await
        .unwrap();
    recv(&mut ws).await;
    reconnected.close().await;
}
#[tokio::test]
async fn upgrade_rejects_bad_key_or_missing_extension() {
    for missing_extension in [false, true] {
        let adapter = AmazonA2aAdapter::new(config()).unwrap();
        let (client, server) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move { adapter.accept_websocket(server, binding()).await });
        let mut req = "ws://localhost/a2a".into_client_request().unwrap();
        req.headers_mut().insert(
            "authorization",
            format!(
                "Bearer {}",
                if missing_extension {
                    KEY
                } else {
                    "incorrect-secret"
                }
            )
            .parse()
            .unwrap(),
        );
        if !missing_extension {
            req.headers_mut()
                .insert("a2a-extensions", EXTENSION.parse().unwrap());
        }
        assert!(tokio_tungstenite::client_async(req, client).await.is_err());
        assert!(task.await.unwrap().is_err());
    }
}
#[tokio::test]
async fn shutdown_retires_live_routes() {
    let (adapter, call, _socket) = start(true).await;
    timeout(Duration::from_secs(2), adapter.shutdown())
        .await
        .unwrap();
    assert!(!adapter.is_connection_live(call.connection_id()));
    call.closed().await;
}
#[test]
fn traces_reject_invalid_context_and_timing_and_secrets_are_redacted() {
    assert!(TraceContext::from_traceparent(
        "00-00000000000000000000000000000000-1234567890abcdef-01"
    )
    .is_err());
    assert!(TurnTrace::new("agent", "input", "output", 20, 10).is_err());
    assert!(trace::ToolTrace::new("tool", "in", "out", 0, 10).is_err());
    assert!(!format!("{:?}", config()).contains(KEY));
}

#[tokio::test]
async fn source_receiver_closes_when_session_ends_even_with_retained_handles() {
    let (adapter, call, _socket) = start(true).await;
    let stream = adapter
        .streams(call.connection_id().clone())
        .await
        .unwrap()
        .remove(0);
    let mut receiver = stream.try_frames_in().unwrap();
    call.close().await;
    assert!(timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .is_none());
    assert!(!stream.source_ready());
}
#[tokio::test]
async fn unaccepted_and_not_ready_connections_have_deadlines() {
    let mut cfg = config();
    cfg.initialization_timeout = Duration::from_millis(50);
    let adapter = AmazonA2aAdapter::new(cfg).unwrap();
    let _events = adapter.subscribe_events();
    let (call, _socket) = handshake(adapter.clone(), true).await;
    timeout(Duration::from_secs(1), call.closed())
        .await
        .unwrap();
    assert!(!adapter.is_connection_live(call.connection_id()));
}
#[tokio::test]
async fn unsupported_finish_is_rejected_without_ending_session() {
    let adapter = AmazonA2aAdapter::new(config()).unwrap();
    let _events = adapter.subscribe_events();
    let (client, server) = tokio::io::duplex(64 * 1024);
    let a = adapter.clone();
    let task = tokio::spawn(async move { a.accept_websocket(server, binding()).await });
    let mut upgrade = "ws://localhost/a2a".into_client_request().unwrap();
    upgrade
        .headers_mut()
        .insert("authorization", format!("Bearer {KEY}").parse().unwrap());
    upgrade
        .headers_mut()
        .insert("a2a-extensions", EXTENSION.parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::client_async(upgrade, client)
        .await
        .unwrap();
    let mut value = init(false);
    value["params"]["message"]["parts"][0]["data"]["initSession"]["supportedFinishTypes"] =
        json!(["COMPLETE"]);
    send(&mut socket, value).await;
    let mut call = task.await.unwrap().unwrap();
    adapter.accept(call.connection_id().clone()).await.unwrap();
    recv(&mut socket).await;
    send(
        &mut socket,
        request(
            Some("CHANNEL_STATE"),
            json!({"data":{"channelState":{"state":"READY"}}}),
        ),
    )
    .await;
    call.next_input().await;
    assert_eq!(
        call.finish(FinishType::Escalate, "human").await,
        Err(A2aError::State)
    );
    call.finish(FinishType::Complete, "done").await.unwrap();
    assert_eq!(
        recv(&mut socket).await["result"]["message"]["parts"][0]["data"]["finish"]["type"],
        "COMPLETE"
    );
    call.closed().await;
}

#[tokio::test]
async fn native_orchestrator_bridges_connect_to_existing_vapi_adapter() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use rvoip_core::{
        commands::InboundAction, config::Config as CoreConfig, conversation::ConversationPolicy,
        events::Event, ids::TenantId, session::SessionMedium, Orchestrator,
    };
    use rvoip_vapi::{
        VapiAdapter, VapiApiKey, VapiAssistant, VapiAudioFormat, VapiCallOptions, VapiConfig,
        VapiExistingCall, VapiPeerFailurePolicy,
    };
    let core = Orchestrator::new(CoreConfig::default());
    let adapter = AmazonA2aAdapter::new(config()).unwrap();
    core.register(adapter.clone()).unwrap();
    let mut events = core.subscribe_events();
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
    let bind = CallBinding {
        session_id: session.clone(),
        ..binding()
    };
    let (mut call, mut socket) = handshake_bound(adapter.clone(), true, bind).await;
    timeout(Duration::from_secs(2),async {loop {if matches!(events.recv().await.unwrap(),Event::ConnectionInbound{connection_id,..} if &connection_id==call.connection_id()){break;}}}).await.unwrap();
    core.route_inbound_connection(
        call.connection_id().clone(),
        InboundAction::Accept {
            session_id: session,
            participant_id: ParticipantId::new(),
        },
    )
    .await
    .unwrap();
    recv(&mut socket).await;
    send(
        &mut socket,
        request(
            Some("CHANNEL_STATE"),
            json!({"data":{"channelState":{"state":"READY"}}}),
        ),
    )
    .await;
    assert!(matches!(call.next_input().await, Some(InputEvent::Ready)));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (observed_tx, mut observed) = tokio::sync::mpsc::channel(4);
    let peer = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        while let Some(Ok(msg)) = ws.next().await {
            match msg {
                Message::Binary(bytes) => {
                    let _ = observed_tx.try_send(bytes.clone());
                    ws.send(Message::Binary(bytes)).await.unwrap();
                }
                Message::Text(text) if text.contains("end-call") => {
                    let _ = ws.close(None).await;
                    break;
                }
                Message::Close(_) => break,
                _ => (),
            }
        }
    });
    let mut vapi_config = VapiConfig::existing_calls_only().with_loopback_test_transport();
    vapi_config.graceful_shutdown_timeout = Duration::from_millis(100);
    let vapi = VapiAdapter::new(vapi_config).unwrap();
    let options = VapiCallOptions::new(VapiAssistant::saved("test-assistant"))
        .with_audio_format(VapiAudioFormat::PcmS16Le16Khz)
        .with_peer_failure_policy(VapiPeerFailurePolicy::LeaveCallerConnected);
    let existing = VapiExistingCall::new(
        options,
        "verified-test-call".into(),
        url::Url::parse(&format!("ws://{address}/audio")).unwrap(),
    )
    .unwrap()
    .with_api_key(VapiApiKey::new("test-vapi-key").unwrap());
    let agent = vapi
        .attach_existing_agent(&core, call.connection_id().clone(), existing)
        .await
        .unwrap();
    let pcm = vec![3u8; 640];
    send(
        &mut socket,
        request(
            None,
            json!({"raw":STANDARD.encode(&pcm),"mediaType":"audio/lpcm"}),
        ),
    )
    .await;
    let received = timeout(Duration::from_secs(3), observed.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.as_ref(), pcm);
    let response = recv(&mut socket).await;
    assert_eq!(
        response["result"]["artifactUpdate"]["artifact"]["parts"][0]["raw"],
        STANDARD.encode(&pcm)
    );
    call.complete_turn(trace()).await.unwrap();
    recv(&mut socket).await;
    recv(&mut socket).await;
    recv(&mut socket).await;
    call.finish(FinishType::Complete, "done").await.unwrap();
    recv(&mut socket).await;
    call.closed().await;
    timeout(Duration::from_secs(3), agent.wait_shared())
        .await
        .unwrap();
    timeout(Duration::from_secs(2), peer)
        .await
        .unwrap()
        .unwrap();
    core.close_conversation(conversation, true).await.unwrap();
}

#[tokio::test]
async fn text_connections_do_not_advertise_a_phantom_audio_stream() {
    let (adapter, call, _socket) = start(false).await;
    assert!(adapter
        .streams(call.connection_id().clone())
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        call.send_audio(Bytes::from_static(&[0, 0])).await,
        Err(A2aError::Protocol)
    );
    call.close().await;
}

#[tokio::test(start_paused = true)]
async fn stalled_reader_cannot_hold_an_audio_command_or_shutdown_forever() {
    let mut cfg = config();
    cfg.write_timeout = Duration::from_secs(1);
    let (adapter, call, _socket) = start_with(cfg, true).await;
    let before = tokio::time::Instant::now();
    assert_eq!(
        call.send_audio(Bytes::from(vec![0; 1024 * 1024])).await,
        Err(A2aError::Timeout)
    );
    call.closed().await;
    assert!(before.elapsed() <= Duration::from_secs(3));
    assert!(!adapter.is_connection_live(call.connection_id()));
}
#[tokio::test]
async fn smallest_frame_budget_never_fragments_24khz_audio() {
    let mut cfg = config();
    cfg.max_frame_bytes = 2048;
    cfg.sample_rate_hz = 24000;
    let adapter = AmazonA2aAdapter::new(cfg).unwrap();
    let _events = adapter.subscribe_events();
    let (mut call, mut socket) = handshake(adapter.clone(), true).await;
    adapter.accept(call.connection_id().clone()).await.unwrap();
    recv(&mut socket).await;
    send(
        &mut socket,
        request(
            Some("CHANNEL_STATE"),
            json!({"data":{"channelState":{"state":"READY"}}}),
        ),
    )
    .await;
    call.next_input().await;
    call.send_audio(Bytes::from(vec![0; 4800])).await.unwrap();
    for _ in 0..5 {
        let value = recv(&mut socket).await;
        assert!(value.to_string().len() <= 2048);
        assert_eq!(
            value["result"]["artifactUpdate"]["artifact"]["metadata"][EVENT_TYPE],
            "AUDIO_RESPONSE_CHUNK"
        );
    }
    call.close().await;
}

#[tokio::test]
async fn malformed_and_unknown_method_requests_use_json_rpc_error_codes() {
    for (text, code, id) in [
        ("{".to_owned(), -32700, Value::Null),
        (
            json!({"jsonrpc":"2.0","id":99,"method":"GetTask","params":{}}).to_string(),
            -32601,
            json!(99),
        ),
    ] {
        let (_, call, mut socket) = start(false).await;
        socket.send(Message::Text(text.into())).await.unwrap();
        let error = recv(&mut socket).await;
        assert_eq!(error["error"]["code"], code);
        assert_eq!(error["id"], id);
        call.closed().await;
    }
}

/// Live Connect's HANDOFF initialization carries contactArn but no instanceArn.
/// Admission must still compare its enclosing instance against trusted policy.
#[tokio::test]
async fn live_init_resolves_contact_instance_and_rejects_cross_instance_binding() {
    let instance =
        "arn:aws:connect:us-east-1:123456789012:instance/11111111-1111-4111-8111-111111111111";
    for allowed in [true, false] {
        let adapter = AmazonA2aAdapter::new(config()).unwrap();
        let _events = adapter.subscribe_events();
        let mut bind = binding();
        bind.instance_arn = if allowed {
            instance.into()
        } else {
            "another-instance".into()
        };
        let (client, server) = tokio::io::duplex(256 * 1024);
        let server_adapter = adapter.clone();
        let task = tokio::spawn(async move { server_adapter.accept_websocket(server, bind).await });
        let mut upgrade = "ws://localhost/a2a".into_client_request().unwrap();
        upgrade
            .headers_mut()
            .insert("authorization", format!("Bearer {KEY}").parse().unwrap());
        upgrade
            .headers_mut()
            .insert("a2a-extensions", EXTENSION.parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::client_async(upgrade, client)
            .await
            .unwrap();
        send(
            &mut socket,
            request(
                Some("INIT_SESSION"),
                json!({"data":{"initSession":{
            "interactionMode":"HANDOFF",
            "contactArn":format!("{instance}/contact/22222222-2222-4222-8222-222222222222"),
            "supportedFinishTypes":["COMPLETE","ESCALATE"],
            "contactContext":{"systemAttributes":{"Channel":"CHAT"}}
        }},"mediaType":"application/json"}),
            ),
        )
        .await;
        let result = task.await.unwrap();
        if allowed {
            let call = result.unwrap();
            assert_eq!(call.initialization().instance_arn, instance);
            adapter.accept(call.connection_id().clone()).await.unwrap();
            assert_eq!(
                recv(&mut socket).await["result"]["message"]["parts"][0]["data"]
                    ["initSessionResponse"]["statusCode"],
                "200"
            );
            call.close().await;
        } else {
            assert!(matches!(result, Err(A2aError::Configuration)));
        }
    }
}
