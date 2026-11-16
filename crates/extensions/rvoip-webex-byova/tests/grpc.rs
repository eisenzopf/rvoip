//! gRPC transport through the rvoip core: per-RPC authentication, one
//! connection per conversation across RPCs, placeholder RPCs, and reply
//! framing on the oldest open RPC.
#![cfg(feature = "grpc")]

mod common;

use bytes::Bytes;
use common::*;
use rvoip_core::{
    adapter::ConnectionAdapter,
    ids::StreamId,
    stream::{MediaFrame, StreamKind},
};
use rvoip_webex_byova::{
    grpc::wire::{
        event_input, voice_input, voice_va_request::VoiceVaInputType,
        voice_va_response::ResponseType, voice_virtual_agent_client::VoiceVirtualAgentClient,
        DtmfInputs, EventInput, ListVaRequest, VoiceInput, VoiceVaRequest, VoiceVaResponse,
    },
    ReplyAction, ReplyBoundary, ReplyMark, WebexByovaAdapter, WebexByovaCall, WebexInput,
};
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::timeout};
use tonic::{transport::Channel, Request, Status, Streaming};

const CONVERSATION: &str = "grpc-conversation-1";

async fn serve(
    adapter: &Arc<WebexByovaAdapter>,
) -> (
    VoiceVirtualAgentClient<Channel>,
    mpsc::Receiver<WebexByovaCall>,
) {
    let (service, calls) = adapter.grpc_service();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let incoming = tonic::transport::server::TcpIncoming::from_listener(listener, true, None)
        .expect("incoming");
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service.into_server())
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });
    let client = VoiceVirtualAgentClient::connect(format!("http://{address}"))
        .await
        .expect("client");
    (client, calls)
}

fn request(input: VoiceVaInputType) -> VoiceVaRequest {
    VoiceVaRequest {
        conversation_id: CONVERSATION.into(),
        customer_org_id: ORG.into(),
        virtual_agent_id: AGENT.into(),
        voice_va_input_type: Some(input),
        ..Default::default()
    }
}

fn event(kind: event_input::EventType) -> VoiceVaRequest {
    request(VoiceVaInputType::EventInput(EventInput {
        event_type: kind as i32,
        ..Default::default()
    }))
}

fn audio(bytes: Vec<u8>) -> VoiceVaRequest {
    request(VoiceVaInputType::AudioInput(VoiceInput {
        caller_audio: bytes,
        encoding: voice_input::VoiceEncoding::MulawFormat as i32,
        sample_rate_hertz: 8000,
        language_code: "en-US".into(),
        ..Default::default()
    }))
}

/// Cisco omits identity and format on later audio frames of one RPC.
fn bare_audio(bytes: Vec<u8>) -> VoiceVaRequest {
    VoiceVaRequest {
        voice_va_input_type: Some(VoiceVaInputType::AudioInput(VoiceInput {
            caller_audio: bytes,
            ..Default::default()
        })),
        ..Default::default()
    }
}

struct Rpc {
    input: mpsc::Sender<VoiceVaRequest>,
    output: Streaming<VoiceVaResponse>,
}

async fn open(
    client: &mut VoiceVirtualAgentClient<Channel>,
    first: VoiceVaRequest,
    token: &str,
) -> Result<Rpc, Status> {
    let (input, receiver) = mpsc::channel(16);
    input.send(first).await.unwrap();
    let stream = futures::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    });
    let mut request = Request::new(stream);
    request
        .metadata_mut()
        .insert("authorization", token.parse().unwrap());
    let output = client.process_caller_input(request).await?.into_inner();
    Ok(Rpc { input, output })
}

async fn next(rpc: &mut Rpc) -> Option<VoiceVaResponse> {
    timeout(Duration::from_secs(5), rpc.output.message())
        .await
        .expect("gRPC response arrived")
        .expect("gRPC stream healthy")
}

async fn reply(rpc: &mut Rpc) -> Vec<u8> {
    let mut audio = Vec::new();
    loop {
        let response = next(rpc).await.expect("reply before end of stream");
        for prompt in &response.prompts {
            audio.extend_from_slice(&prompt.audio_content);
        }
        if response.response_type == ResponseType::Final as i32 {
            // FINAL completes this RPC's response.
            assert!(next(rpc).await.is_none());
            return audio;
        }
    }
}

async fn agent_audio(adapter: &WebexByovaAdapter, call: &WebexByovaCall, bytes: Vec<u8>) {
    let stream = adapter
        .streams(call.connection_id().clone())
        .await
        .unwrap()
        .remove(0);
    stream
        .try_frames_out()
        .expect("agent audio sender")
        .send(MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from(bytes),
            timestamp_rtp: 0,
            captured_at: chrono::Utc::now(),
            payload_type: Some(0),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn every_rpc_requires_a_valid_grpc_registration_token() {
    let adapter = WebexByovaAdapter::new(config()).unwrap();
    let (mut client, _calls) = serve(&adapter).await;
    for token in [
        "not-a-token".to_owned(),
        // A WebSocket registration's token cannot use gRPC.
        common::token(false),
    ] {
        let status = open(
            &mut client,
            event(event_input::EventType::SessionStart),
            &format!("Bearer {token}"),
        )
        .await
        .err()
        .expect("rejected");
        assert_eq!(status.code(), tonic::Code::Unauthenticated);
    }
    let mut list = Request::new(ListVaRequest {
        customer_org_id: ORG.into(),
        is_default_virtual_agent_enabled: false,
    });
    list.metadata_mut()
        .insert("authorization", common::token(true).parse().unwrap());
    let agents = client.list_virtual_agents(list).await.unwrap().into_inner();
    assert_eq!(agents.virtual_agents[0].virtual_agent_id, AGENT);
    let mut other = Request::new(ListVaRequest {
        customer_org_id: "another-org".into(),
        is_default_virtual_agent_enabled: false,
    });
    other
        .metadata_mut()
        .insert("authorization", common::token(true).parse().unwrap());
    assert_eq!(
        client.list_virtual_agents(other).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
    // A conversation cannot begin without SESSION_START.
    let status = open(&mut client, audio(vec![0xff; 320]), &common::token(true))
        .await
        .err()
        .expect("rejected");
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    assert_eq!(adapter.connection_count(), 0);
}

#[tokio::test]
async fn one_connection_spans_interactions_across_rpcs() {
    let adapter = WebexByovaAdapter::new(config()).unwrap();
    let _events = adapter.subscribe_events();
    let (mut client, mut calls) = serve(&adapter).await;
    let token = common::token(true);
    let mut first = open(
        &mut client,
        event(event_input::EventType::SessionStart),
        &token,
    )
    .await
    .unwrap();
    let mut call = timeout(Duration::from_secs(2), calls.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.conversation_id(), CONVERSATION);
    adapter.accept(call.connection_id().clone()).await.unwrap();
    assert!(matches!(call.next_input().await, Some(WebexInput::Started)));
    let stream = adapter
        .streams(call.connection_id().clone())
        .await
        .unwrap()
        .remove(0);
    let mut caller = stream.try_frames_in().expect("caller audio");

    // Greeting on the SESSION_START RPC: one-second CHUNKs, then the tail
    // with FINAL, which completes that RPC.
    let greeting: Vec<u8> = (0..8_100u32).map(|n| (n % 251) as u8).collect();
    agent_audio(&adapter, &call, greeting.clone()).await;
    call.complete_reply(ReplyBoundary::At(8_100)).await.unwrap();
    assert_eq!(reply(&mut first).await, greeting);

    // A listening RPC starts with Cisco's placeholder; later frames omit
    // identity and format. Caller audio reaches the media stream unchanged.
    let mut second = open(&mut client, audio(vec![0xff; 3]), &token)
        .await
        .unwrap();
    let speech = vec![0x5a; 320];
    second.input.send(bare_audio(speech.clone())).await.unwrap();
    let frame = timeout(Duration::from_secs(2), caller.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.payload.as_ref(), speech);
    second
        .input
        .send(request(VoiceVaInputType::DtmfInput(DtmfInputs {
            dtmf_events: vec![1, 2, 16],
        })))
        .await
        .unwrap();
    assert!(matches!(call.next_input().await, Some(WebexInput::Dtmf(digits)) if digits == "12#"));

    // The answer goes to the RPC that carried the input.
    // `Received` would race the frame into the session task; the exact
    // position names the reply's last byte.
    agent_audio(&adapter, &call, vec![0x42; 200]).await;
    call.complete_reply(ReplyBoundary::At(8_300)).await.unwrap();
    assert_eq!(reply(&mut second).await, vec![0x42; 200]);

    // A placeholder-only RPC that half-closes completes without ending the
    // conversation.
    let Rpc { input, output } = open(&mut client, audio(vec![0xff; 3]), &token)
        .await
        .unwrap();
    drop(input);
    let mut idle = Rpc {
        input: mpsc::channel(1).0,
        output,
    };
    assert!(next(&mut idle).await.is_none());
    assert!(adapter.is_connection_live(call.connection_id()));

    // SESSION_END on a new RPC ends the conversation and its connection.
    let _last = open(
        &mut client,
        event(event_input::EventType::SessionEnd),
        &token,
    )
    .await
    .unwrap();
    assert!(matches!(call.next_input().await, Some(WebexInput::Ended)));
    timeout(Duration::from_secs(3), call.closed())
        .await
        .unwrap();
    assert!(!adapter.is_connection_live(call.connection_id()));
    assert_eq!(adapter.connection_count(), 0);
}

#[tokio::test]
async fn transfer_request_is_one_terminal_final() {
    let adapter = WebexByovaAdapter::new(config()).unwrap();
    let _events = adapter.subscribe_events();
    let (mut client, mut calls) = serve(&adapter).await;
    let token = common::token(true);
    let mut rpc = open(
        &mut client,
        event(event_input::EventType::SessionStart),
        &token,
    )
    .await
    .unwrap();
    let mut call = calls.recv().await.unwrap();
    adapter.accept(call.connection_id().clone()).await.unwrap();
    assert!(matches!(call.next_input().await, Some(WebexInput::Started)));
    call.transfer_to_agent().await.unwrap();
    let response = next(&mut rpc).await.unwrap();
    assert_eq!(response.response_type, ResponseType::Final as i32);
    assert_eq!(
        response.output_events[0].event_type,
        rvoip_webex_byova::grpc::wire::output_event::EventType::TransferToAgent as i32
    );
    assert!(next(&mut rpc).await.is_none());
    timeout(Duration::from_secs(3), call.closed())
        .await
        .unwrap();
}

/// An in-band mark splits one queued frame at its exact byte. The second
/// reply has no open RPC, so it waits for the next one.
#[tokio::test]
async fn reply_marks_split_a_frame_and_output_waits_for_the_next_rpc() {
    let adapter = WebexByovaAdapter::new(config()).unwrap();
    let _events = adapter.subscribe_events();
    let (mut client, mut calls) = serve(&adapter).await;
    let token = common::token(true);
    let mut first = open(
        &mut client,
        event(event_input::EventType::SessionStart),
        &token,
    )
    .await
    .unwrap();
    let mut call = calls.recv().await.unwrap();
    adapter.accept(call.connection_id().clone()).await.unwrap();
    assert!(matches!(call.next_input().await, Some(WebexInput::Started)));
    let (marks, receiver) = mpsc::channel(4);
    call.follow_reply_marks(futures::stream::unfold(
        receiver,
        |mut receiver| async move { receiver.recv().await.map(|mark| (mark, receiver)) },
    ))
    .await
    .unwrap();
    marks
        .send(ReplyMark {
            position: 150,
            action: ReplyAction::Complete,
        })
        .await
        .unwrap();
    let mut frame = vec![0x01; 150];
    frame.extend_from_slice(&[0x02; 150]);
    agent_audio(&adapter, &call, frame).await;
    assert_eq!(reply(&mut first).await, vec![0x01; 150]);

    // The rest of the frame forms the next reply, held until an RPC opens.
    call.complete_reply(ReplyBoundary::At(300)).await.unwrap();
    let mut second = open(&mut client, audio(vec![0x7f; 160]), &token)
        .await
        .unwrap();
    assert_eq!(reply(&mut second).await, vec![0x02; 150]);
    call.close().await;
    assert_eq!(adapter.connection_count(), 0);
}
