//! Webex BYOVA over gRPC (`grpc` feature).
//!
//! Webex opens a new `ProcessCallerInput` RPC for each interaction of one
//! conversation. The adapter keeps one rvoip connection per conversation and
//! attaches each RPC to it:
//!
//! - the first RPC must start the conversation with SESSION_START; it is
//!   authenticated, admitted and published to core exactly like a WebSocket
//!   conversation;
//! - agent output goes to the oldest open RPC. Its FINAL completes that RPC's
//!   response normally; the conversation continues on the next RPC;
//! - meaningful input (caller audio longer than Cisco's 15-byte listening
//!   placeholder, digits or an event) on a newer RPC supersedes older ones,
//!   cancelling any reply still streaming on them;
//! - a placeholder-only RPC whose input ends without output completes
//!   normally without ending the conversation;
//! - client cancellation of an RPC that is carrying a reply ends the
//!   conversation; SESSION_END, a terminal reply, `close`, or core teardown
//!   end it as well;
//! - agent output produced while no RPC is open waits, bounded by
//!   [`crate::WebexByovaConfig::max_pending_output_bytes`], for the next RPC.
//!
//! Every RPC is authenticated with the `authorization` metadata before its
//! body is read.

use crate::{
    adapter::{
        Admitted, AgentAudio, Command, CommandRequest, Marks, PendingCompletion, ReplyAction,
        ReplyMark, RouteGuard, WebexByovaAdapter, WebexByovaCall, WebexInput,
    },
    auth::{bearer, WebexPrincipal, WebexTransportKind},
    grpc::{
        self,
        wire::{
            event_input, output_event, text_content, voice_input, voice_va_request,
            voice_va_response,
            voice_virtual_agent_server::{VoiceVirtualAgent, VoiceVirtualAgentServer},
            ListVaRequest, ListVaResponse, OutputEvent, TextContent, VirtualAgentInfo,
            VoiceVaRequest, VoiceVaResponse,
        },
        ChunkReply,
    },
    media::{AudioReceivers, PCMU_PAYLOAD_TYPE},
    Result, WebexError, WebexGrpcReplyFraming,
};
use bytes::Bytes;
use futures::{stream::SelectAll, Stream, StreamExt};
use rvoip_core::{ids::ConnectionId, stream::MediaFrame};
use serde_json::Value;
use std::{collections::VecDeque, pin::Pin, sync::Arc};
use tokio::{
    sync::mpsc,
    time::{timeout, Instant},
};
use tokio_util::sync::CancellationToken;
use tonic::{metadata::MetadataMap, Request, Response, Status, Streaming};

const RESPONSE_QUEUE: usize = 32;
const MAX_ENCODED_RESPONSE_BYTES: usize = 256 * 1024;
const PLACEHOLDER_MAX_BYTES: usize = 15;

type ResponseStream =
    Pin<Box<dyn Stream<Item = std::result::Result<VoiceVaResponse, Status>> + Send>>;

/// One RPC handed to its conversation's session task.
pub(crate) struct RpcAttachment {
    first: VoiceVaRequest,
    input: Streaming<VoiceVaRequest>,
    output: mpsc::Sender<std::result::Result<VoiceVaResponse, Status>>,
}

/// The tonic service for Cisco's `VoiceVirtualAgent` contract.
///
/// Create it with [`WebexByovaAdapter::grpc_service`] and serve
/// [`Self::into_server`] behind TLS. New conversations arrive on the paired
/// receiver as [`WebexByovaCall`]s; the host must consume it.
#[derive(Clone)]
pub struct WebexByovaGrpcService {
    adapter: Arc<WebexByovaAdapter>,
    calls: mpsc::Sender<WebexByovaCall>,
}

impl WebexByovaAdapter {
    /// The gRPC service and the receiver of its new conversations.
    pub fn grpc_service(
        self: &Arc<Self>,
    ) -> (WebexByovaGrpcService, mpsc::Receiver<WebexByovaCall>) {
        let (calls, receiver) = mpsc::channel(self.config.queue_capacity);
        (
            WebexByovaGrpcService {
                adapter: self.clone(),
                calls,
            },
            receiver,
        )
    }
}

impl WebexByovaGrpcService {
    /// The tonic server with Cisco's message bounds: 128 KiB requests and
    /// 256 KiB responses.
    pub fn into_server(self) -> VoiceVirtualAgentServer<Self> {
        VoiceVirtualAgentServer::new(self)
            .max_decoding_message_size(grpc::MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_ENCODED_RESPONSE_BYTES)
    }

    fn authenticate(&self, metadata: &MetadataMap) -> std::result::Result<WebexPrincipal, Status> {
        let token = metadata
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| bearer(value, WebexTransportKind::Grpc))
            .ok_or_else(|| Status::unauthenticated("authentication required"))?;
        self.adapter
            .config
            .authenticator
            .authenticate(token, WebexTransportKind::Grpc)
            .map_err(|error| match error {
                WebexError::KeysUnavailable => Status::unavailable("authentication unavailable"),
                _ => Status::unauthenticated("authentication failed"),
            })
    }
}

impl std::fmt::Debug for WebexByovaGrpcService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WebexByovaGrpcService")
    }
}

fn is_session_start(request: &VoiceVaRequest) -> bool {
    matches!(&request.voice_va_input_type,
        Some(voice_va_request::VoiceVaInputType::EventInput(event))
        if event.event_type == event_input::EventType::SessionStart as i32)
}

fn start_parameters(request: &VoiceVaRequest) -> std::result::Result<Value, Status> {
    let parameters = match &request.voice_va_input_type {
        Some(voice_va_request::VoiceVaInputType::EventInput(event)) => {
            event.parameters.as_ref().map(struct_to_json)
        }
        _ => None,
    };
    let parameters = parameters.unwrap_or(Value::Null);
    if parameters.to_string().len() > 16_384 {
        return Err(Status::invalid_argument("start parameters exceed 16 KiB"));
    }
    Ok(parameters)
}

#[tonic::async_trait]
impl VoiceVirtualAgent for WebexByovaGrpcService {
    type ProcessCallerInputStream = ResponseStream;

    async fn process_caller_input(
        &self,
        request: Request<Streaming<VoiceVaRequest>>,
    ) -> std::result::Result<Response<Self::ProcessCallerInputStream>, Status> {
        let principal = self.authenticate(request.metadata())?;
        let config = self.adapter.config.clone();
        let mut input = request.into_inner();
        let first = timeout(config.start_timeout, input.message())
            .await
            .map_err(|_| Status::deadline_exceeded("no request"))??
            .ok_or_else(|| Status::invalid_argument("empty request stream"))?;
        grpc::validate_request(&first, &principal.organization_id, &config.virtual_agent.id)
            .map_err(Status::invalid_argument)?;
        let key = (
            principal.organization_id.clone(),
            first.conversation_id.clone(),
        );
        let (output, receiver) = mpsc::channel(RESPONSE_QUEUE);
        let stream: ResponseStream = Box::pin(futures::stream::unfold(
            receiver,
            |mut receiver| async move { receiver.recv().await.map(|item| (item, receiver)) },
        ));
        let attachment = RpcAttachment {
            first,
            input,
            output,
        };

        let existing = self
            .adapter
            .conversations
            .get(&key)
            .map(|entry| entry.clone())
            .and_then(|id| self.adapter.routes.get(&id).map(|route| route.clone()));
        if let Some(route) = existing {
            if is_session_start(&attachment.first) {
                return Err(Status::already_exists("conversation already started"));
            }
            route
                .rpcs
                .as_ref()
                .ok_or_else(|| Status::failed_precondition("conversation uses another transport"))?
                .try_send(attachment)
                .map_err(|_| Status::resource_exhausted("conversation is busy"))?;
            return Ok(Response::new(stream));
        }
        if !is_session_start(&attachment.first) {
            return Err(Status::failed_precondition("conversation not started"));
        }
        let parameters = start_parameters(&attachment.first)?;
        let permit = self
            .adapter
            .acquire_slot()
            .map_err(|_| Status::resource_exhausted("capacity"))?;
        let admitted = self
            .adapter
            .admit(&principal, &key.1, WebexTransportKind::Grpc, parameters)
            .map_err(|error| match error {
                WebexError::DuplicateConversation => {
                    Status::already_exists("conversation already started")
                }
                _ => Status::unavailable("admission unavailable"),
            })?;
        let Admitted {
            route,
            call,
            audio,
            commands,
            inputs,
            rpcs,
        } = admitted;
        let Some(rpcs) = rpcs else {
            return Err(Status::internal("conversation attachment unavailable"));
        };
        let adapter = self.adapter.clone();
        let guard = RouteGuard {
            adapter: adapter.clone(),
            id: call.connection_id.clone(),
            route: route.clone(),
            permit: Some(permit),
        };
        let organization = principal.organization_id.clone();
        tokio::spawn(async move {
            let guard = guard;
            let mut actor = Actor::new(
                adapter.clone(),
                guard.route.clone(),
                organization,
                key.1,
                audio,
                inputs,
            );
            let result = actor.run(&guard.id, attachment, rpcs, commands).await;
            actor.close_all();
            adapter.retire(&guard.id, &guard.route, result).await;
            drop(guard);
        });
        if self.calls.try_send(call).is_err() {
            // The host is not taking new conversations: fail closed.
            route.cancel.cancel();
            return Err(Status::unavailable("host is not accepting conversations"));
        }
        Ok(Response::new(stream))
    }

    async fn list_virtual_agents(
        &self,
        request: Request<ListVaRequest>,
    ) -> std::result::Result<Response<ListVaResponse>, Status> {
        let principal = self.authenticate(request.metadata())?;
        let request = request.into_inner();
        if request.customer_org_id != principal.organization_id {
            return Err(Status::permission_denied("organization mismatch"));
        }
        let agent = &self.adapter.config.virtual_agent;
        let virtual_agents = if request.is_default_virtual_agent_enabled && !agent.is_default {
            vec![]
        } else {
            vec![VirtualAgentInfo {
                virtual_agent_id: agent.id.clone(),
                virtual_agent_name: agent.name.clone(),
                is_default: agent.is_default,
                attributes: Default::default(),
            }]
        };
        Ok(Response::new(ListVaResponse { virtual_agents }))
    }
}

enum Out {
    Response(VoiceVaResponse),
    /// The reply is final: complete the carrying RPC's response.
    EndReply,
}

struct Rpc {
    id: u64,
    output: mpsc::Sender<std::result::Result<VoiceVaResponse, Status>>,
    cancel: CancellationToken,
    meaningful: bool,
    /// Identity and audio format from this RPC's first request. Cisco omits
    /// them on later audio frames of the same RPC.
    customer_org_id: String,
    audio_format: Option<(i32, i32, String)>,
}

type RpcInput = (u64, Option<std::result::Result<VoiceVaRequest, Status>>);
type Inputs = SelectAll<Pin<Box<dyn Stream<Item = RpcInput> + Send>>>;

struct Actor {
    adapter: Arc<WebexByovaAdapter>,
    route: Arc<crate::adapter::Route>,
    organization: String,
    conversation: String,
    audio_rx: AudioReceivers,
    inputs: mpsc::Sender<WebexInput>,
    rpcs: VecDeque<Rpc>,
    streams: Inputs,
    next_rpc: u64,
    reply: Option<ChunkReply>,
    backlog: VecDeque<Out>,
    backlog_bytes: usize,
    audio: AgentAudio,
    marks: Marks,
    reply_bytes: usize,
    caller_timestamp: u32,
}

enum Step {
    Continue,
    Finished,
}

impl Actor {
    fn new(
        adapter: Arc<WebexByovaAdapter>,
        route: Arc<crate::adapter::Route>,
        organization: String,
        conversation: String,
        audio_rx: AudioReceivers,
        inputs: mpsc::Sender<WebexInput>,
    ) -> Self {
        Self {
            adapter,
            route,
            organization,
            conversation,
            audio_rx,
            inputs,
            rpcs: VecDeque::new(),
            streams: SelectAll::new(),
            next_rpc: 0,
            reply: None,
            backlog: VecDeque::new(),
            backlog_bytes: 0,
            audio: AgentAudio::default(),
            marks: Marks::default(),
            reply_bytes: 0,
            caller_timestamp: 0,
        }
    }

    fn deliver(&self, input: WebexInput) -> Result<()> {
        self.inputs
            .try_send(input)
            .map_err(|_| WebexError::Capacity)
    }

    fn close_all(&mut self) {
        for rpc in self.rpcs.drain(..) {
            rpc.cancel.cancel();
        }
    }

    async fn run(
        &mut self,
        id: &ConnectionId,
        first: RpcAttachment,
        mut attachments: mpsc::Receiver<RpcAttachment>,
        mut commands: mpsc::Receiver<CommandRequest>,
    ) -> Result<()> {
        let config = self.adapter.config.clone();
        let route = self.route.clone();
        route.wait_accepted(config.admission_timeout).await?;
        route
            .stream
            .active
            .store(true, std::sync::atomic::Ordering::Release);
        self.adapter.publish_connected(id)?;
        self.deliver(WebexInput::Started)?;
        // SESSION_START itself is consumed by admission; the RPC stays open
        // to carry the greeting.
        self.attach(first, None).await?;

        let mut pending: Option<PendingCompletion> = None;
        let session_deadline = Instant::now() + config.max_session_duration;
        loop {
            let completion_deadline = pending
                .as_ref()
                .map(|p| p.deadline)
                .or_else(|| self.marks.deadline(config.reply_boundary_timeout));
            tokio::select! {
                _ = route.cancel.cancelled() => {
                    let _ = self.terminal(false).await;
                    return Ok(());
                }
                _ = tokio::time::sleep_until(session_deadline) => return Err(WebexError::Timeout),
                _ = async {
                    match completion_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                }, if completion_deadline.is_some() => {
                    if let Some(pending) = pending.take() {
                        let _ = pending.reply.send(Err(WebexError::Timeout));
                    }
                    return Err(WebexError::Timeout);
                }
                Some(attachment) = attachments.recv() => {
                    if self.attach_and_process(attachment).await? {
                        let _ = self.deliver(WebexInput::Ended);
                        return Ok(());
                    }
                }
                _ = self.marks.next(), if self.marks.is_following() => {
                    while let Some(mark) = self.marks.due(self.audio.received) {
                        let items = self.mark_items(mark)?;
                        self.emit(items).await?;
                    }
                }
                Some(request) = commands.recv(), if pending.is_none() => {
                    if let Step::Finished = self.command(request, &mut pending).await? {
                        return Ok(());
                    }
                }
                Some((rpc, item)) = self.streams.next(), if !self.streams.is_empty() => {
                    if let Step::Finished = self.input(rpc, item)? {
                        return Ok(());
                    }
                }
                Some(frame) = self.audio_rx.peer.recv() => {
                    if let Some((frame, _guard)) = frame.into_delivery() {
                        self.agent_audio(frame).await?;
                    }
                    self.settle(&mut pending).await?;
                }
                Some(frame) = self.audio_rx.outbound.recv() => {
                    self.agent_audio(frame).await?;
                    self.settle(&mut pending).await?;
                }
            }
        }
    }

    /// Attach a later RPC and process its first request. Returns true for
    /// SESSION_END.
    async fn attach_and_process(&mut self, attachment: RpcAttachment) -> Result<bool> {
        let id = self.attach(attachment, Some(())).await?;
        match id {
            Some((id, first)) => self.request(id, first),
            None => Ok(false),
        }
    }

    /// Attach an RPC. A later RPC's first request is returned for
    /// processing; the conversation's first RPC (SESSION_START) expects the
    /// greeting, so it counts as meaningful input.
    async fn attach(
        &mut self,
        attachment: RpcAttachment,
        later: Option<()>,
    ) -> Result<Option<(u64, VoiceVaRequest)>> {
        let RpcAttachment {
            first,
            input,
            output,
        } = attachment;
        let id = self.next_rpc;
        self.next_rpc += 1;
        let cancel = CancellationToken::new();
        let audio_format = match &first.voice_va_input_type {
            Some(voice_va_request::VoiceVaInputType::AudioInput(audio)) => Some((
                audio.encoding,
                audio.sample_rate_hertz,
                audio.language_code.clone(),
            )),
            _ => None,
        };
        self.rpcs.push_back(Rpc {
            id,
            output,
            cancel: cancel.clone(),
            meaningful: later.is_none(),
            customer_org_id: first.customer_org_id.clone(),
            audio_format,
        });
        let stream = input
            .take_until(cancel.cancelled_owned())
            .map(move |item| (id, Some(item)))
            .chain(futures::stream::once(async move { (id, None) }));
        self.streams.push(Box::pin(stream));
        // Output that waited for an RPC goes to the oldest one.
        let backlog: Vec<Out> = self.backlog.drain(..).collect();
        self.backlog_bytes = 0;
        self.emit(backlog).await?;
        Ok(later.map(|()| (id, first)))
    }

    fn input(
        &mut self,
        rpc: u64,
        item: Option<std::result::Result<VoiceVaRequest, Status>>,
    ) -> Result<Step> {
        let Some(position) = self.rpcs.iter().position(|current| current.id == rpc) else {
            return Ok(Step::Continue);
        };
        match item {
            Some(Ok(request)) => {
                if self.request(rpc, request)? {
                    let _ = self.deliver(WebexInput::Ended);
                    return Ok(Step::Finished);
                }
                Ok(Step::Continue)
            }
            Some(Err(_)) => {
                // Cancelling the RPC that carries a reply ends the conversation.
                if position == 0 && self.reply.is_some() {
                    return Err(WebexError::Closed);
                }
                if let Some(rpc) = self.rpcs.remove(position) {
                    rpc.cancel.cancel();
                }
                Ok(Step::Continue)
            }
            None => {
                // Input ended. An RPC that carried no meaningful input and is
                // not carrying a reply completes normally; others stay open
                // for their reply.
                let carrying = position == 0 && self.reply.is_some();
                if !self.rpcs[position].meaningful && !carrying {
                    if let Some(rpc) = self.rpcs.remove(position) {
                        rpc.cancel.cancel();
                    }
                }
                Ok(Step::Continue)
            }
        }
    }

    /// Validate and route one request. Returns true for SESSION_END.
    fn request(&mut self, rpc: u64, mut request: VoiceVaRequest) -> Result<bool> {
        let Some(position) = self.rpcs.iter().position(|current| current.id == rpc) else {
            return Ok(false);
        };
        {
            let current = &self.rpcs[position];
            if request.customer_org_id.is_empty() {
                request.customer_org_id = current.customer_org_id.clone();
            }
            if request.conversation_id.is_empty() {
                request.conversation_id = self.conversation.clone();
            }
            if let (
                Some(voice_va_request::VoiceVaInputType::AudioInput(audio)),
                Some((encoding, rate, language)),
            ) = (&mut request.voice_va_input_type, &current.audio_format)
            {
                if audio.encoding == voice_input::VoiceEncoding::UnspecifiedFormat as i32 {
                    audio.encoding = *encoding;
                }
                if audio.sample_rate_hertz == 0 {
                    audio.sample_rate_hertz = *rate;
                }
                if audio.language_code.is_empty() {
                    audio.language_code = language.clone();
                }
            }
        }
        if request.conversation_id != self.conversation {
            return Err(WebexError::Protocol);
        }
        grpc::validate_request(
            &request,
            &self.organization,
            &self.adapter.config.virtual_agent.id,
        )
        .map_err(|_| WebexError::Protocol)?;
        let input = request.voice_va_input_type.ok_or(WebexError::Protocol)?;
        let meaningful = !matches!(&input,
            voice_va_request::VoiceVaInputType::AudioInput(audio)
            if audio.caller_audio.len() <= PLACEHOLDER_MAX_BYTES);
        if meaningful {
            self.supersede(position);
        }
        match input {
            voice_va_request::VoiceVaInputType::AudioInput(audio) => {
                if meaningful {
                    let len = audio.caller_audio.len() as u32;
                    self.route
                        .stream
                        .input(Bytes::from(audio.caller_audio), self.caller_timestamp)?;
                    self.caller_timestamp = self.caller_timestamp.wrapping_add(len);
                }
            }
            voice_va_request::VoiceVaInputType::DtmfInput(digits) => {
                let digits = grpc::dtmf_text(&digits).map_err(|_| WebexError::Protocol)?;
                self.deliver(WebexInput::Dtmf(digits))?;
            }
            voice_va_request::VoiceVaInputType::EventInput(event) => {
                match event_input::EventType::try_from(event.event_type) {
                    Ok(event_input::EventType::SessionEnd) => return Ok(true),
                    Ok(event_input::EventType::NoInput) => self.deliver(WebexInput::NoInput)?,
                    Ok(event_input::EventType::StartOfDtmf) => {
                        self.deliver(WebexInput::DtmfStarted)?
                    }
                    Ok(event_input::EventType::CustomEvent) => {
                        if event.name.is_empty() || event.name.len() > 128 {
                            return Err(WebexError::Protocol);
                        }
                        let parameters = event
                            .parameters
                            .as_ref()
                            .map(struct_to_json)
                            .unwrap_or(Value::Null);
                        self.deliver(WebexInput::Custom {
                            name: event.name,
                            parameters,
                        })?;
                    }
                    _ => return Err(WebexError::State),
                }
            }
        }
        Ok(false)
    }

    /// Meaningful input on `position` retires every older RPC; a reply still
    /// streaming on them is cancelled.
    fn supersede(&mut self, position: usize) {
        self.rpcs[position].meaningful = true;
        if position == 0 {
            return;
        }
        for rpc in self.rpcs.drain(..position) {
            let _ = rpc
                .output
                .try_send(Err(Status::cancelled("superseded by newer input")));
            rpc.cancel.cancel();
        }
        self.reply = None;
    }

    async fn emit(&mut self, items: Vec<Out>) -> Result<()> {
        let write_timeout = self.adapter.config.write_timeout;
        for item in items {
            let Some(front) = self.rpcs.front() else {
                let bytes = match &item {
                    Out::Response(response) => response
                        .prompts
                        .iter()
                        .map(|prompt| prompt.audio_content.len())
                        .sum::<usize>()
                        .max(1),
                    Out::EndReply => 0,
                };
                self.backlog_bytes = self.backlog_bytes.saturating_add(bytes);
                if self.backlog_bytes > self.adapter.config.max_pending_output_bytes {
                    return Err(WebexError::Capacity);
                }
                self.backlog.push_back(item);
                continue;
            };
            match item {
                Out::Response(response) => {
                    timeout(write_timeout, front.output.send(Ok(response)))
                        .await
                        .map_err(|_| WebexError::Timeout)?
                        .map_err(|_| WebexError::Closed)?;
                }
                Out::EndReply => {
                    if let Some(rpc) = self.rpcs.pop_front() {
                        // Dropping the sender completes the response stream.
                        rpc.cancel.cancel();
                    }
                }
            }
        }
        Ok(())
    }

    fn framing(&self) -> ChunkReply {
        match self.adapter.config.grpc_reply_framing {
            WebexGrpcReplyFraming::StreamingChunks => ChunkReply::default(),
            _ => ChunkReply::one_second(),
        }
    }

    /// Write one agent frame, applying every in-band mark at its exact byte.
    async fn agent_audio(&mut self, frame: MediaFrame) -> Result<()> {
        if frame.payload_type != Some(PCMU_PAYLOAD_TYPE) {
            return Err(WebexError::Protocol);
        }
        // Marks published before this audio are already queued: take them first.
        self.marks.drain_ready();
        let mut rest: &[u8] = &frame.payload;
        loop {
            while let Some(mark) = self.marks.due(self.audio.received) {
                let items = self.mark_items(mark)?;
                self.emit(items).await?;
            }
            if rest.is_empty() {
                break;
            }
            let take = self.marks.before_next(self.audio.received, rest.len());
            let fresh = self.audio.accept(&rest[..take]).to_vec();
            rest = &rest[take..];
            if fresh.is_empty() {
                continue;
            }
            if self.reply.is_none() {
                self.reply = Some(self.framing());
                self.reply_bytes = 0;
            }
            self.reply_bytes = self.reply_bytes.saturating_add(fresh.len());
            let responses = match self.reply.as_mut() {
                Some(reply) => reply.push(&fresh).map_err(|_| WebexError::Capacity)?,
                None => Vec::new(),
            };
            self.emit(responses.into_iter().map(Out::Response).collect())
                .await?;
        }
        self.marks.audio_progressed();
        Ok(())
    }

    fn mark_items(&mut self, mark: ReplyMark) -> Result<Vec<Out>> {
        match mark.action {
            ReplyAction::Complete => match self.finish_reply() {
                // Shorter than Cisco's minimum chunk: keep the reply open.
                Err(WebexError::ShortAudio) => Ok(Vec::new()),
                result => result,
            },
            ReplyAction::Interrupt => Ok(self.interrupt_items()),
        }
    }

    fn interrupt_items(&mut self) -> Vec<Out> {
        let mut items = vec![Out::Response(grpc::event(
            output_event::EventType::StartOfInput,
            false,
        ))];
        if self.reply.take().is_some() {
            items.push(Out::Response(grpc::response(
                voice_va_response::ResponseType::Final,
            )));
            items.push(Out::EndReply);
        }
        items
    }

    /// The tail and FINAL of the open reply, or one empty FINAL. A reply
    /// shorter than Cisco's minimum chunk stays open.
    fn finish_reply(&mut self) -> Result<Vec<Out>> {
        if self.reply.is_some() && self.reply_bytes < grpc::MIN_CHUNK_BYTES {
            return Err(WebexError::ShortAudio);
        }
        let mut items = Vec::new();
        match self.reply.take() {
            Some(mut reply) => {
                let responses = reply.finish().map_err(|_| WebexError::ShortAudio)?;
                items.extend(responses.into_iter().map(Out::Response));
            }
            None => items.push(Out::Response(grpc::response(
                voice_va_response::ResponseType::Final,
            ))),
        }
        items.push(Out::EndReply);
        Ok(items)
    }

    async fn settle(&mut self, pending: &mut Option<PendingCompletion>) -> Result<()> {
        self.route
            .received
            .store(self.audio.received, std::sync::atomic::Ordering::Release);
        if pending
            .as_ref()
            .is_some_and(|pending| self.audio.received >= pending.position)
        {
            if let Some(pending) = pending.take() {
                let result = match self.finish_reply() {
                    Ok(items) => self.emit(items).await,
                    Err(error) => Err(error),
                };
                let fatal = matches!(result, Err(WebexError::Closed | WebexError::Timeout));
                let _ = pending.reply.send(result);
                if fatal {
                    return Err(WebexError::Closed);
                }
            }
        }
        Ok(())
    }

    async fn terminal(&mut self, handoff: bool) -> Result<()> {
        let mut items = Vec::new();
        if let Some(mut reply) = self.reply.take() {
            if let Ok(responses) = reply.finish() {
                // The reply's own FINAL is replaced by the terminal FINAL.
                items.extend(
                    responses
                        .into_iter()
                        .filter(|response| {
                            response.response_type != voice_va_response::ResponseType::Final as i32
                        })
                        .map(Out::Response),
                );
            }
        }
        let kind = if handoff {
            output_event::EventType::TransferToAgent
        } else {
            output_event::EventType::SessionEnd
        };
        items.push(Out::Response(grpc::event(kind, true)));
        items.push(Out::EndReply);
        self.emit(items).await
    }

    async fn command(
        &mut self,
        request: CommandRequest,
        pending: &mut Option<PendingCompletion>,
    ) -> Result<Step> {
        let CommandRequest { command, reply } = request;
        let mut step = Step::Continue;
        let result = match command {
            Command::CompleteReply(boundary) => {
                let position = self.audio.position(boundary);
                if self.audio.received < position {
                    *pending = Some(PendingCompletion {
                        position,
                        reply,
                        deadline: Instant::now() + self.adapter.config.reply_boundary_timeout,
                    });
                    return Ok(Step::Continue);
                }
                match self.finish_reply() {
                    Ok(items) => self.emit(items).await,
                    Err(error) => Err(error),
                }
            }
            Command::Interrupt(boundary) => {
                self.audio.fence(boundary);
                let items = self.interrupt_items();
                self.emit(items).await
            }
            Command::FollowMarks(source) => self.marks.follow(source),
            Command::Event(event, transcript) => {
                let kind = match event {
                    crate::WebexOutputEvent::EndOfInput => output_event::EventType::EndOfInput,
                    crate::WebexOutputEvent::NoMatch => output_event::EventType::NoMatch,
                    crate::WebexOutputEvent::NoInput => output_event::EventType::NoInput,
                };
                let mut response = grpc::event(kind, false);
                match transcript {
                    Some(text) if text.len() > 8192 => Err(WebexError::Capacity),
                    transcript => {
                        response.session_transcript = transcript.map(|text| TextContent {
                            input_content: Some(text_content::InputContent::Text(text)),
                            ..Default::default()
                        });
                        self.emit(vec![Out::Response(response)]).await
                    }
                }
            }
            Command::Terminal { handoff } => {
                step = Step::Finished;
                self.terminal(handoff).await
            }
            Command::CompleteWithMetadata { name, metadata } => {
                if name.is_empty()
                    || name.len() > 128
                    || name.chars().any(char::is_control)
                    || !metadata.is_object()
                    || metadata.to_string().len() > 16_384
                {
                    Err(WebexError::Capacity)
                } else {
                    step = Step::Finished;
                    let fields = json_to_struct(&metadata);
                    let mut items = Vec::new();
                    if let Some(mut reply) = self.reply.take() {
                        if let Ok(responses) = reply.finish() {
                            items.extend(
                                responses
                                    .into_iter()
                                    .filter(|response| {
                                        response.response_type
                                            != voice_va_response::ResponseType::Final as i32
                                    })
                                    .map(Out::Response),
                            );
                        }
                    }
                    let mut response = grpc::response(voice_va_response::ResponseType::Final);
                    response.output_events = vec![
                        OutputEvent {
                            event_type: output_event::EventType::CustomEvent as i32,
                            name,
                            metadata: Some(fields.clone()),
                        },
                        OutputEvent {
                            event_type: output_event::EventType::SessionEnd as i32,
                            name: String::new(),
                            metadata: Some(fields),
                        },
                    ];
                    items.push(Out::Response(response));
                    items.push(Out::EndReply);
                    self.emit(items).await
                }
            }
        };
        let fatal = matches!(
            result,
            Err(WebexError::Closed | WebexError::Timeout | WebexError::Capacity)
        );
        let _ = reply.send(result);
        if fatal {
            return Err(WebexError::Closed);
        }
        Ok(step)
    }
}

fn struct_to_json(value: &prost_types::Struct) -> Value {
    Value::Object(
        value
            .fields
            .iter()
            .map(|(key, value)| (key.clone(), prost_to_json(value)))
            .collect(),
    )
}

fn prost_to_json(value: &prost_types::Value) -> Value {
    use prost_types::value::Kind;
    match &value.kind {
        Some(Kind::BoolValue(value)) => Value::Bool(*value),
        Some(Kind::NumberValue(value)) => serde_json::Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Some(Kind::StringValue(value)) => Value::String(value.clone()),
        Some(Kind::StructValue(value)) => struct_to_json(value),
        Some(Kind::ListValue(list)) => {
            Value::Array(list.values.iter().map(prost_to_json).collect())
        }
        Some(Kind::NullValue(_)) | None => Value::Null,
    }
}

fn json_to_struct(value: &Value) -> prost_types::Struct {
    prost_types::Struct {
        fields: value
            .as_object()
            .map(|object| {
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), json_to_prost(value)))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn json_to_prost(value: &Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match value {
        Value::Null => Kind::NullValue(0),
        Value::Bool(value) => Kind::BoolValue(*value),
        Value::Number(value) => Kind::NumberValue(value.as_f64().unwrap_or_default()),
        Value::String(value) => Kind::StringValue(value.clone()),
        Value::Array(values) => Kind::ListValue(prost_types::ListValue {
            values: values.iter().map(json_to_prost).collect(),
        }),
        Value::Object(_) => Kind::StructValue(json_to_struct(value)),
    };
    prost_types::Value { kind: Some(kind) }
}
