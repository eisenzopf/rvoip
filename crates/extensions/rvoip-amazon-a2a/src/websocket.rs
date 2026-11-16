use crate::{
    adapter::{AmazonA2aAdapter, AmazonA2aCall, CallBinding, Command, InputEvent, Route},
    fragment::{self, Reassembler},
    media::{self, AudioReceivers, AudioStream},
    protocol::{self, InterruptionType, Request},
    session::Session,
    trace::TraceContext,
    A2aError, Result,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::BytesMut;
use futures::{SinkExt, StreamExt};
use rvoip_core::{
    adapter::{AdapterEvent, EndReason},
    connection::{Connection, ConnectionState, Direction, Transport, TransportHandle},
    ids::ConnectionId,
    stream::MediaStreamHandle,
    NegotiatedCodecs,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, OwnedSemaphorePermit},
    time::{timeout, Instant},
};
use tokio_tungstenite::{
    accept_hdr_async_with_config,
    tungstenite::{
        handshake::server::{
            ErrorResponse, Request as UpgradeRequest, Response as UpgradeResponse,
        },
        http::StatusCode,
        protocol::WebSocketConfig,
        Message as WsMessage,
    },
    WebSocketStream,
};
use tokio_util::sync::CancellationToken;

impl AmazonA2aAdapter {
    /// Authenticate and accept an A2A WebSocket on an already secured stream.
    ///
    /// Production callers MUST supply TLS or a private connection from their
    /// TLS-terminating ingress. This method checks the bearer key and extension
    /// header even behind that ingress. It never installs a plaintext listener.
    /// The returned connection must be accepted through the Orchestrator; the
    /// INIT_SESSION response is withheld until that acceptance.
    pub async fn accept_websocket<S>(
        self: &Arc<Self>,
        io: S,
        binding: CallBinding,
    ) -> Result<AmazonA2aCall>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| A2aError::Capacity)?;
        let config = self.config.clone();
        let captured = Arc::new(parking_lot::Mutex::new(None));
        let context_slot = captured.clone();
        let callback = move |request: &UpgradeRequest,
                             mut response: UpgradeResponse|
              -> std::result::Result<UpgradeResponse, ErrorResponse> {
            let reject = |status| {
                let mut r = ErrorResponse::new(Some("A2A upgrade rejected".into()));
                *r.status_mut() = status;
                r
            };
            let token = request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "));
            if !token.is_some_and(|t| config.api_key.matches(t)) {
                return Err(reject(StatusCode::UNAUTHORIZED));
            }
            let extensions = request
                .headers()
                .get("a2a-extensions")
                .and_then(|v| v.to_str().ok());
            if !extensions.is_some_and(|v| v.split(',').any(|s| s.trim() == protocol::EXTENSION)) {
                return Err(reject(StatusCode::BAD_REQUEST));
            }
            let trace = match request.headers().get("traceparent") {
                Some(value) => value
                    .to_str()
                    .ok()
                    .and_then(|v| TraceContext::from_traceparent(v).ok())
                    .ok_or_else(|| reject(StatusCode::BAD_REQUEST))?,
                None => TraceContext::root(),
            };
            *context_slot.lock() = Some(trace);
            response.headers_mut().insert(
                "a2a-extensions",
                protocol::EXTENSION
                    .parse()
                    .map_err(|_| reject(StatusCode::INTERNAL_SERVER_ERROR))?,
            );
            Ok(response)
        };
        let ws_config = WebSocketConfig::default()
            .max_message_size(Some(self.config.max_frame_bytes))
            .max_frame_size(Some(self.config.max_frame_bytes));
        let mut socket = timeout(
            self.config.initialization_timeout,
            accept_hdr_async_with_config(io, callback, Some(ws_config)),
        )
        .await
        .map_err(|_| A2aError::Timeout)?
        .map_err(|_| A2aError::Protocol)?;
        let mut fragments = Reassembler::default();
        let request = timeout(self.config.initialization_timeout, async {
            loop {
                let incoming = match fragments.deadline() {
                    Some(deadline) => tokio::time::timeout_at(deadline, socket.next())
                        .await
                        .map_err(|_| A2aError::Timeout)?,
                    None => socket.next().await,
                };
                match incoming {
                    Some(Ok(WsMessage::Text(text))) => match fragments.push(&text, &self.config) {
                        Ok(Some(request)) => return Ok(request),
                        Ok(None) => (),
                        Err(error) => {
                            let _ = send(
                                &mut socket,
                                vec![protocol::rpc_error_from_text(&text)],
                                self,
                            )
                            .await;
                            return Err(error);
                        }
                    },
                    Some(Ok(WsMessage::Ping(_))) => {
                        socket.flush().await.map_err(|_| A2aError::Closed)?
                    }
                    _ => return Err(A2aError::Protocol),
                }
            }
        })
        .await
        .map_err(|_| A2aError::Timeout)??;
        let trace = captured.lock().take().ok_or(A2aError::Trace)?;
        let request_id = request.id.clone();
        let (session, init_reply) = match Session::new(request, trace, self.config.sample_rate_hz) {
            Ok(session) => session,
            Err(error) => {
                let _ = send(&mut socket, vec![protocol::rpc_error(request_id)], self).await;
                return Err(error);
            }
        };
        let _admission = self.admission.lock();
        if binding.instance_arn != session.init.instance_arn || self.slots.is_closed() {
            return Err(A2aError::Configuration);
        }
        let context = session.context.clone();
        let connection_id = ConnectionId::new();
        match self.contexts.entry(context.clone()) {
            dashmap::mapref::entry::Entry::Occupied(_) => return Err(A2aError::DuplicateContext),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(connection_id.clone());
            }
        }
        let cancel = CancellationToken::new();
        let (stream, audio) = AudioStream::new(
            self.config.sample_rate_hz,
            self.config.queue_capacity,
            cancel.clone(),
        );
        let (commands, command_rx) = mpsc::channel(self.config.queue_capacity);
        let (inputs, input_rx) = mpsc::channel(self.config.queue_capacity);
        let route = Arc::new(Route {
            stream: stream.clone(),
            cancel,
            accepted: AtomicBool::new(false),
            acceptance: tokio::sync::Notify::new(),
            finished: CancellationToken::new(),
            commands,
            context,
            audio_enabled: session.init.is_audio(),
        });
        let connection = Connection {
            id: connection_id.clone(),
            session_id: binding.session_id,
            participant_id: binding.participant_id,
            transport: Transport::AmazonA2a,
            direction: Direction::Inbound,
            state: ConnectionState::Connecting,
            capabilities: rvoip_core::CapabilityDescriptor {
                audio_codecs: if session.init.is_audio() {
                    vec![media::codec(self.config.sample_rate_hz)]
                } else {
                    vec![]
                },
                max_streams_per_connection: 1,
                ..Default::default()
            },
            negotiated_codecs: NegotiatedCodecs {
                audio: session
                    .init
                    .is_audio()
                    .then(|| media::codec(self.config.sample_rate_hz)),
                video: None,
            },
            streams: if session.init.is_audio() {
                vec![MediaStreamHandle::new(stream)]
            } else {
                vec![]
            },
            // Text turns require explicit trace-aware call controls.
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: chrono::Utc::now(),
            closed_at: None,
        };
        self.routes.insert(connection_id.clone(), route.clone());
        if self
            .events
            .try_send(AdapterEvent::InboundConnection { connection })
            .is_err()
        {
            self.routes.remove(&connection_id);
            self.contexts.remove(&route.context);
            return Err(A2aError::Capacity);
        }
        let call = AmazonA2aCall {
            connection_id: connection_id.clone(),
            route: route.clone(),
            init: session.init.clone(),
            inputs: input_rx,
            command_timeout: self.config.write_timeout * 4,
        };
        let adapter = self.clone();
        tokio::spawn(async move {
            let guard = RouteGuard {
                adapter: adapter.clone(),
                id: connection_id,
                route: route.clone(),
                _permit: Some(permit),
            };
            let result = run(
                &adapter,
                &guard.id,
                &route,
                &mut socket,
                session,
                init_reply,
                audio,
                command_rx,
                inputs,
            )
            .await;
            route.stream.stop();
            adapter.routes.remove(&guard.id);
            adapter.contexts.remove(&route.context);
            let event = match result {
                Ok(()) => AdapterEvent::Ended {
                    connection_id: guard.id.clone(),
                    reason: EndReason::Normal,
                },
                Err(_) => AdapterEvent::Failed {
                    connection_id: guard.id.clone(),
                    detail: "A2A session failed".into(),
                },
            };
            adapter
                .lifecycle
                .queue_or_deliver_terminal(&adapter.events, event)
                .await;
            let _ = timeout(adapter.config.write_timeout, socket.close(None)).await;
            drop(guard);
        });
        Ok(call)
    }
}
struct RouteGuard {
    adapter: Arc<AmazonA2aAdapter>,
    id: ConnectionId,
    route: Arc<Route>,
    _permit: Option<OwnedSemaphorePermit>,
}
impl Drop for RouteGuard {
    fn drop(&mut self) {
        self.route.stream.stop();
        if self.adapter.routes.remove(&self.id).is_some() {
            self.adapter.contexts.remove(&self.route.context);
            let _ = self.adapter.events.try_send(AdapterEvent::Failed {
                connection_id: self.id.clone(),
                detail: "A2A worker stopped".into(),
            });
        }
        self._permit.take();
        self.route.finished.cancel();
    }
}
async fn send<S>(
    socket: &mut WebSocketStream<S>,
    values: Vec<Value>,
    adapter: &AmazonA2aAdapter,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let frames: Vec<String> = values
        .into_iter()
        .map(|v| fragment::encode(v, &adapter.config))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    timeout(adapter.config.write_timeout, async {
        for frame in frames {
            socket
                .send(WsMessage::Text(frame.into()))
                .await
                .map_err(|_| A2aError::Closed)?;
        }
        Ok(())
    })
    .await
    .map_err(|_| A2aError::Timeout)?
}
async fn output_audio<S>(
    socket: &mut WebSocketStream<S>,
    session: &mut Session,
    pcm: &[u8],
    adapter: &AmazonA2aAdapter,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if pcm.is_empty() || pcm.len() % 2 != 0 || pcm.len() > adapter.config.max_payload_bytes {
        return Err(A2aError::Protocol);
    }
    // Independent <=20ms chunks fit the frame budget; audio is NEVER fragmented.
    let chunk_bytes = adapter.config.sample_rate_hz as usize / 50 * 2;
    let frames = pcm
        .chunks(chunk_bytes)
        .map(|chunk| session.audio(chunk))
        .collect::<Result<Vec<_>>>()?;
    send(socket, frames, adapter).await
}
async fn drain_audio<S>(
    socket: &mut WebSocketStream<S>,
    session: &mut Session,
    audio: &mut AudioReceivers,
    adapter: &AmazonA2aAdapter,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Snapshot the boundary: a producer that has not stopped cannot keep this
    // loop alive indefinitely. The public API requires upstream quiescence.
    for _ in 0..audio.outbound.len() {
        if let Ok(frame) = audio.outbound.try_recv() {
            output_audio(socket, session, &frame.payload, adapter).await?;
        }
    }
    for _ in 0..audio.peer.len() {
        if let Ok(frame) = audio.peer.try_recv() {
            if let Some((frame, _guard)) = frame.into_delivery() {
                output_audio(socket, session, &frame.payload, adapter).await?;
            }
        }
    }
    Ok(())
}
fn discard_audio(audio: &mut AudioReceivers) {
    while audio.outbound.try_recv().is_ok() {}
    while audio.peer.try_recv().is_ok() {}
}
#[allow(clippy::too_many_arguments)]
async fn run<S>(
    adapter: &AmazonA2aAdapter,
    id: &ConnectionId,
    route: &Arc<Route>,
    socket: &mut WebSocketStream<S>,
    mut session: Session,
    init_reply: Value,
    mut audio: AudioReceivers,
    mut commands: mpsc::Receiver<crate::adapter::CommandRequest>,
    inputs: mpsc::Sender<InputEvent>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let accepted = async {
        loop {
            let notified = route.acceptance.notified();
            if route.accepted.load(Ordering::Acquire) {
                break;
            }
            notified.await;
        }
    };
    tokio::select! { _=route.cancel.cancelled()=>return Ok(()), result=timeout(adapter.config.initialization_timeout,accepted)=>{result.map_err(|_|A2aError::Timeout)?;} }
    send(socket, vec![init_reply], adapter).await?;
    let mut fragments = Reassembler::default();
    let mut input_buffer = BytesMut::new();
    let mut timestamp = 0u32;
    let mut deadline = Instant::now() + adapter.config.initialization_timeout;
    loop {
        let fragment_deadline = fragments.deadline().unwrap_or(deadline);
        tokio::select! {
            _=route.cancel.cancelled()=>return Ok(()),
            _=tokio::time::sleep_until(deadline.min(fragment_deadline))=>{let failure=session.failure();let _=send(socket,vec![failure],adapter).await;return Err(A2aError::Timeout);}
            Some(request)=commands.recv()=>{
                let result = tokio::select! {
                    _ = route.cancel.cancelled() => Err(A2aError::Closed),
                    result = timeout(adapter.config.write_timeout, command(
                        request.command, socket, &mut session, &mut audio, adapter
                    )) => result.unwrap_or(Err(A2aError::Timeout)),
                };
                if result.is_ok() && session.ready { deadline=Instant::now()+adapter.config.idle_timeout; }
                let fatal=matches!(result,Err(A2aError::Closed|A2aError::Timeout|A2aError::Capacity));
                let _=request.reply.send(result);
                if fatal {return Err(A2aError::Closed);}
                if session.finished {return Ok(());}
            }
            incoming=socket.next()=>{
                match incoming {
                    Some(Ok(WsMessage::Text(text)))=>{
                        let parsed=fragments.push(&text,&adapter.config);
                        let request=match parsed {Ok(Some(r))=>r,Ok(None)=>continue,Err(e)=>{let _=send(socket,vec![protocol::rpc_error_from_text(&text)],adapter).await;return Err(e);}};
                        let request_id=request.id.clone();
                        let result=receive(request,&mut session,route,&inputs,&mut input_buffer,&mut timestamp,&mut audio,adapter,id);
                        if let Err(error)=result {let _=send(socket,vec![protocol::rpc_error(request_id)],adapter).await;return Err(error);}
                        if session.ready { deadline=Instant::now()+adapter.config.idle_timeout; }
                    },
                    Some(Ok(WsMessage::Ping(_)))=>{timeout(adapter.config.write_timeout,socket.flush()).await.map_err(|_|A2aError::Timeout)?.map_err(|_|A2aError::Closed)?;},
                    Some(Ok(WsMessage::Pong(_)))=>(),
                    Some(Ok(WsMessage::Close(_)))|None=>return Ok(()),
                    _=>return Err(A2aError::Protocol),
                }
            }
            Some(frame)=audio.outbound.recv(),if session.ready=>{
                if !session.turn.as_ref().is_some_and(|t|t.interrupted) {output_audio(socket,&mut session,&frame.payload,adapter).await?;}
            }
            Some(frame)=audio.peer.recv(),if session.ready=>{
                if !session.turn.as_ref().is_some_and(|t|t.interrupted) {if let Some((frame,_guard))=frame.into_delivery(){output_audio(socket,&mut session,&frame.payload,adapter).await?;}}
            }
        }
    }
}
async fn command<S>(
    command: Command,
    socket: &mut WebSocketStream<S>,
    session: &mut Session,
    audio: &mut AudioReceivers,
    adapter: &AmazonA2aAdapter,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match command {
        Command::Audio(pcm) => output_audio(socket, session, &pcm, adapter).await,
        Command::Text(text) => {
            let frame = session.text(text)?;
            send(socket, vec![frame], adapter).await
        }
        Command::Complete(trace) => {
            if session.turn.as_ref().is_some_and(|turn| turn.interrupted) {
                discard_audio(audio);
            } else {
                drain_audio(socket, session, audio, adapter).await?;
            }
            let frames = session.complete(trace)?;
            send(socket, frames, adapter).await
        }
        Command::Interrupt(kind) => {
            discard_audio(audio);
            let frames = session.interrupt(kind)?;
            send(socket, frames, adapter).await
        }
        Command::Finish(kind, reason) => {
            let frame = session.finish(kind, reason)?;
            send(socket, vec![frame], adapter).await
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn receive(
    request: Request,
    session: &mut Session,
    route: &Route,
    inputs: &mpsc::Sender<InputEvent>,
    buffer: &mut BytesMut,
    timestamp: &mut u32,
    audio: &mut AudioReceivers,
    adapter: &AmazonA2aAdapter,
    id: &ConnectionId,
) -> Result<()> {
    if request.params.message.context_id != session.context || session.finished {
        return Err(A2aError::Protocol);
    }
    let deliver = |event| inputs.try_send(event).map_err(|_| A2aError::Capacity);
    match request.event() {
        Some("CHANNEL_STATE") => {
            #[derive(Deserialize)]
            struct Channel {
                state: String,
            }
            let state: Channel = request.data("channelState")?;
            if state.state != "READY" || session.ready {
                return Err(A2aError::State);
            }
            session.ready = true;
            route
                .stream
                .active
                .store(session.init.is_audio(), Ordering::Release);
            adapter
                .events
                .try_send(AdapterEvent::Connected {
                    connection_id: id.clone(),
                })
                .map_err(|_| A2aError::Capacity)?;
            deliver(InputEvent::Ready)?;
        }
        Some("INTERRUPTION") => {
            if !session.ready {
                return Err(A2aError::State);
            }
            #[derive(Deserialize)]
            struct Interruption {
                r#type: InterruptionType,
            }
            let interruption: Interruption = request.data("interruption")?;
            session.peer_interrupt();
            discard_audio(audio);
            deliver(InputEvent::Interrupted(interruption.r#type))?;
        }
        Some("DTMF_INPUT_COMPLETE") => {
            if !session.ready {
                return Err(A2aError::State);
            }
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Dtmf {
                input_string: String,
                termination_reason: String,
            }
            let dtmf: Dtmf = request.data("dtmfInputComplete")?;
            if dtmf.input_string.len() > 1024
                || !dtmf
                    .input_string
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b"*#ABCD".contains(&b))
            {
                return Err(A2aError::Protocol);
            }
            deliver(InputEvent::Dtmf {
                digits: dtmf.input_string,
                termination_reason: dtmf.termination_reason,
            })?;
        }
        Some("ERROR") => deliver(InputEvent::Diagnostic(request.data("error")?))?,
        Some(_) => return Err(A2aError::Unsupported),
        None => {
            if !session.ready {
                return Err(A2aError::State);
            }
            for part in request.params.message.parts {
                match (part.raw, part.text, part.data) {
                    (Some(raw), None, None)
                        if session.init.is_audio()
                            && part.media_type.as_deref() == Some("audio/lpcm") =>
                    {
                        let bytes = STANDARD.decode(raw).map_err(|_| A2aError::Protocol)?;
                        if bytes.is_empty() || bytes.len() % 2 != 0 {
                            return Err(A2aError::Protocol);
                        }
                        buffer.extend_from_slice(&bytes);
                        let frame_bytes = adapter.config.sample_rate_hz as usize / 50 * 2;
                        while buffer.len() >= frame_bytes {
                            route
                                .stream
                                .input(buffer.split_to(frame_bytes).freeze(), *timestamp)?;
                            *timestamp = timestamp.wrapping_add(frame_bytes as u32 / 2);
                        }
                    }
                    (None, Some(text), None) => deliver(InputEvent::Text(text))?,
                    _ => return Err(A2aError::Protocol),
                }
            }
        }
    }
    Ok(())
}
