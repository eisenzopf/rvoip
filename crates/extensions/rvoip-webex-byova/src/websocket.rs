//! Webex BYOVA over WebSocket (`/v1/va` and `/v1/listVirtualAgents`).

use crate::{
    adapter::{
        Admitted, AgentAudio, Command, CommandRequest, Marks, PendingCompletion, ReplyAction,
        ReplyMark, RouteGuard, WebexByovaAdapter, WebexByovaCall, WebexInput,
    },
    auth::{bearer, WebexPrincipal, WebexTransportKind},
    media::{AudioReceivers, PCMU_PAYLOAD_TYPE},
    protocol::Input,
    session::Session,
    Result, WebexError,
};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rvoip_core::{ids::ConnectionId, stream::MediaFrame};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
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

/// What an accepted WebSocket turned out to be.
#[derive(Debug)]
pub enum WebexWebSocket {
    /// An admitted conversation, published to core and awaiting acceptance.
    Conversation(WebexByovaCall),
    /// An authenticated list-agents request, answered and closed.
    AgentsListed,
}

#[derive(Clone, Copy, PartialEq)]
enum Endpoint {
    Conversation,
    ListAgents,
}

impl WebexByovaAdapter {
    /// Authenticate and accept one Webex WebSocket on an already secured
    /// stream.
    ///
    /// Production hosts terminate TLS (Webex requires WSS) and pass the
    /// decrypted stream; this method never opens a listener. The upgrade is
    /// rejected with 401 unless its `Authorization: Bearer` token passes the
    /// configured [`crate::WebexAuthenticator`], and with 404 for any path
    /// other than the configured conversation and list paths. A conversation
    /// is published to core only after a valid SESSION_START from the
    /// authenticated organization, together with its principal, and receives
    /// no reply until core accepts it.
    pub async fn accept_websocket<S>(self: &Arc<Self>, io: S) -> Result<WebexWebSocket>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let permit = self.acquire_slot()?;
        let config = self.config.clone();
        let captured = Arc::new(parking_lot::Mutex::new(None::<(Endpoint, WebexPrincipal)>));
        let slot = captured.clone();
        let callback = move |request: &UpgradeRequest,
                             response: UpgradeResponse|
              -> std::result::Result<UpgradeResponse, ErrorResponse> {
            let reject = |status| {
                let mut error = ErrorResponse::new(Some("Webex upgrade rejected".into()));
                *error.status_mut() = status;
                error
            };
            let path = request.uri().path();
            let endpoint = if path == config.websocket_path {
                Endpoint::Conversation
            } else if path == config.list_agents_path {
                Endpoint::ListAgents
            } else {
                return Err(reject(StatusCode::NOT_FOUND));
            };
            let token = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| bearer(value, WebexTransportKind::WebSocket))
                .ok_or_else(|| reject(StatusCode::UNAUTHORIZED))?;
            let principal = config
                .authenticator
                .authenticate(token, WebexTransportKind::WebSocket)
                .map_err(|error| {
                    reject(if error == WebexError::KeysUnavailable {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::UNAUTHORIZED
                    })
                })?;
            *slot.lock() = Some((endpoint, principal));
            Ok(response)
        };
        let ws_config = WebSocketConfig::default()
            .max_message_size(Some(self.config.max_frame_bytes))
            .max_frame_size(Some(self.config.max_frame_bytes));
        let mut socket = timeout(
            self.config.start_timeout,
            accept_hdr_async_with_config(io, callback, Some(ws_config)),
        )
        .await
        .map_err(|_| WebexError::Timeout)?
        .map_err(|_| WebexError::Unauthorized)?;
        let (endpoint, principal) = captured.lock().take().ok_or(WebexError::Unauthorized)?;
        if endpoint == Endpoint::ListAgents {
            let result = self.list_agents(&mut socket, &principal).await;
            let _ = timeout(self.config.write_timeout, socket.close(None)).await;
            return result.map(|()| WebexWebSocket::AgentsListed);
        }

        let mut session = Session::new(
            principal.organization_id.clone(),
            self.config.virtual_agent.id.clone(),
        )
        .with_audio_output(self.config.audio_output);
        // Before SESSION_START nothing is admitted; the first application
        // message must be a valid start from the authenticated organization.
        let start = timeout(self.config.start_timeout, async {
            loop {
                match socket.next().await {
                    Some(Ok(WsMessage::Text(text))) => return session.receive(&text),
                    Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => continue,
                    _ => return Err(crate::protocol::ProtocolError::Envelope),
                }
            }
        })
        .await
        .map_err(|_| WebexError::Timeout)?;
        if !matches!(start, Ok(Input::Start)) {
            let _ = timeout(self.config.write_timeout, socket.close(None)).await;
            return Err(WebexError::Protocol);
        }
        let conversation = session
            .conversation_id()
            .ok_or(WebexError::Protocol)?
            .to_owned();
        let admitted = match self.admit(
            &principal,
            &conversation,
            WebexTransportKind::WebSocket,
            session.start_parameters().clone(),
        ) {
            Ok(admitted) => admitted,
            Err(error) => {
                if let Ok(message) = session.error("rate_limit", 429) {
                    let _ = send(&mut socket, vec![message], &self.config).await;
                }
                let _ = timeout(self.config.write_timeout, socket.close(None)).await;
                return Err(error);
            }
        };
        let Admitted {
            route,
            call,
            audio,
            commands,
            inputs,
            ..
        } = admitted;
        let adapter = self.clone();
        let guard = RouteGuard {
            adapter: adapter.clone(),
            id: call.connection_id.clone(),
            route,
            permit: Some(permit),
        };
        tokio::spawn(async move {
            let guard = guard;
            let result = run(
                &adapter,
                &guard.id,
                &mut socket,
                session,
                audio,
                commands,
                inputs,
            )
            .await;
            adapter.retire(&guard.id, &guard.route, result).await;
            let _ = timeout(adapter.config.write_timeout, socket.close(None)).await;
            drop(guard);
        });
        Ok(WebexWebSocket::Conversation(call))
    }

    async fn list_agents<S>(
        &self,
        socket: &mut WebSocketStream<S>,
        principal: &WebexPrincipal,
    ) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ListRequest {
            customer_org_id: String,
            #[serde(default)]
            is_default_virtual_agent_enabled: bool,
        }
        let text = timeout(self.config.start_timeout, async {
            loop {
                match socket.next().await {
                    Some(Ok(WsMessage::Text(text))) => return Ok(text),
                    Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => continue,
                    _ => return Err(WebexError::Protocol),
                }
            }
        })
        .await
        .map_err(|_| WebexError::Timeout)??;
        let request: ListRequest = serde_json::from_str(&text).map_err(|_| WebexError::Protocol)?;
        // The list is tenant-bound: another organization sees nothing.
        if request.customer_org_id != principal.organization_id {
            return Err(WebexError::Unauthorized);
        }
        let agent = &self.config.virtual_agent;
        let agents = if request.is_default_virtual_agent_enabled && !agent.is_default {
            vec![]
        } else {
            vec![json!({"id": agent.id, "name": agent.name})]
        };
        send(
            socket,
            vec![json!({"virtual_agents": agents})],
            &self.config,
        )
        .await
    }
}

async fn send<S>(
    socket: &mut WebSocketStream<S>,
    values: Vec<Value>,
    config: &crate::WebexByovaConfig,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if values.is_empty() {
        return Ok(());
    }
    timeout(config.write_timeout, async {
        for value in values {
            socket
                .send(WsMessage::Text(value.to_string().into()))
                .await
                .map_err(|_| WebexError::Closed)?;
        }
        Ok(())
    })
    .await
    .map_err(|_| WebexError::Timeout)?
}

/// Write agent audio as it arrives. Webex paces playback, so nothing here
/// waits on a media clock.
fn write_audio(session: &mut Session, audio: &mut AgentAudio, bytes: &[u8]) -> Result<Vec<Value>> {
    let fresh = audio.accept(bytes);
    if fresh.is_empty() {
        return Ok(Vec::new());
    }
    let reply = match session.active_reply() {
        Some(reply) => reply,
        None => session.begin_reply()?,
    };
    Ok(session.audio(reply, fresh)?)
}

/// Apply one in-band mark. A completion that Webex would reject as shorter
/// than its minimum chunk keeps the reply open instead of padding it.
fn apply_mark(session: &mut Session, mark: ReplyMark) -> Result<Vec<Value>> {
    match mark.action {
        ReplyAction::Complete => match finish_reply(session) {
            Err(WebexError::ShortAudio) => {
                tracing::debug!("Webex reply mark left a short reply open");
                Ok(Vec::new())
            }
            result => result,
        },
        ReplyAction::Interrupt => Ok(session.interrupt()?),
    }
}

/// Write one agent frame, applying every mark that falls inside it at its
/// exact byte.
fn agent_audio(
    session: &mut Session,
    audio: &mut AgentAudio,
    marks: &mut Marks,
    frame: MediaFrame,
) -> Result<Vec<Value>> {
    if frame.payload_type != Some(PCMU_PAYLOAD_TYPE) {
        return Err(WebexError::Protocol);
    }
    // Marks published before this audio are already queued: take them first.
    marks.drain_ready();
    let mut output = Vec::new();
    let mut rest: &[u8] = &frame.payload;
    loop {
        while let Some(mark) = marks.due(audio.received) {
            output.extend(apply_mark(session, mark)?);
        }
        if rest.is_empty() {
            break;
        }
        let take = marks.before_next(audio.received, rest.len());
        output.extend(write_audio(session, audio, &rest[..take])?);
        rest = &rest[take..];
    }
    marks.audio_progressed();
    Ok(output)
}

fn finish_reply(session: &mut Session) -> Result<Vec<Value>> {
    let reply = match session.active_reply() {
        Some(reply) => reply,
        // An empty reply still gets exactly one FINAL.
        None => session.begin_reply()?,
    };
    Ok(session.finish(reply)?)
}

enum Step {
    Continue,
    Finished,
}

#[allow(clippy::too_many_arguments)]
async fn run<S>(
    adapter: &Arc<WebexByovaAdapter>,
    id: &ConnectionId,
    socket: &mut WebSocketStream<S>,
    mut session: Session,
    mut audio_rx: AudioReceivers,
    mut commands: mpsc::Receiver<CommandRequest>,
    inputs: mpsc::Sender<WebexInput>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let config = adapter.config.clone();
    let route = adapter
        .routes
        .get(id)
        .map(|route| route.clone())
        .ok_or(WebexError::Closed)?;
    let deliver = |input| inputs.try_send(input).map_err(|_| WebexError::Capacity);
    let mut caller_timestamp = 0u32;

    // Until core accepts, answer liveness and buffer caller audio in the
    // bounded stream queue, but write nothing else.
    let accepted = route.wait_accepted(config.admission_timeout);
    tokio::pin!(accepted);
    loop {
        tokio::select! {
            result = &mut accepted => { result?; break; }
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(WsMessage::Text(text))) => match session.receive(&text) {
                        Ok(Input::Ping(seq)) => send(socket, vec![session.pong(seq)?], &config).await?,
                        Ok(Input::Pong(_)) => {}
                        Ok(Input::Audio(bytes)) => {
                            let len = bytes.len() as u32;
                            route.stream.input(Bytes::from(bytes), caller_timestamp)?;
                            caller_timestamp = caller_timestamp.wrapping_add(len);
                        }
                        Ok(Input::End) => return Ok(()),
                        Ok(_) => return Err(WebexError::State),
                        Err(error) => {
                            let _ = send(socket, vec![session.error(error.code(), 400)?], &config).await;
                            return Err(error.into());
                        }
                    },
                    Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => {}
                    _ => return Err(WebexError::Closed),
                }
            }
        }
    }
    route
        .stream
        .active
        .store(true, std::sync::atomic::Ordering::Release);
    adapter.publish_connected(id)?;
    deliver(WebexInput::Started)?;

    let mut audio = AgentAudio::default();
    let mut marks = Marks::default();
    let mut pending: Option<PendingCompletion> = None;
    let session_deadline = Instant::now() + config.max_session_duration;
    loop {
        let completion_deadline = pending
            .as_ref()
            .map(|p| p.deadline)
            .or_else(|| marks.deadline(config.reply_boundary_timeout));
        tokio::select! {
            _ = route.cancel.cancelled() => {
                // A local end still tells Webex the leg is over.
                if let Ok(messages) = session.terminal(false) {
                    let _ = send(socket, messages, &config).await;
                }
                return Ok(());
            }
            _ = tokio::time::sleep_until(session_deadline) => {
                let _ = send(socket, vec![session.error("timeout", 408)?], &config).await;
                return Err(WebexError::Timeout);
            }
            _ = async {
                match completion_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            }, if completion_deadline.is_some() => {
                if let Some(pending) = pending.take() {
                    let _ = pending.reply.send(Err(WebexError::Timeout));
                }
                let _ = send(socket, vec![session.error("timeout", 504)?], &config).await;
                return Err(WebexError::Timeout);
            }
            _ = marks.next(), if marks.is_following() => {
                let mut messages = Vec::new();
                while let Some(mark) = marks.due(audio.received) {
                    messages.extend(apply_mark(&mut session, mark)?);
                }
                send(socket, messages, &config).await?;
            }
            Some(request) = commands.recv(), if pending.is_none() => {
                match command(request, socket, &mut session, &mut audio, &mut marks, &mut pending, &config).await? {
                    Step::Continue => {}
                    Step::Finished => return Ok(()),
                }
            }
            Some(frame) = audio_rx.peer.recv() => {
                if let Some((frame, _guard)) = frame.into_delivery() {
                    let messages = agent_audio(&mut session, &mut audio, &mut marks, frame)?;
                    send(socket, messages, &config).await?;
                }
                settle(socket, &mut session, &mut audio, &mut pending, &route, &config).await?;
            }
            Some(frame) = audio_rx.outbound.recv() => {
                let messages = agent_audio(&mut session, &mut audio, &mut marks, frame)?;
                send(socket, messages, &config).await?;
                settle(socket, &mut session, &mut audio, &mut pending, &route, &config).await?;
            }
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(WsMessage::Text(text))) => {
                        let input = match session.receive(&text) {
                            Ok(input) => input,
                            Err(error) => {
                                let _ = send(socket, vec![session.error(error.code(), 400)?], &config).await;
                                return Err(error.into());
                            }
                        };
                        match input {
                            Input::Audio(bytes) => {
                                let len = bytes.len() as u32;
                                route.stream.input(Bytes::from(bytes), caller_timestamp)?;
                                caller_timestamp = caller_timestamp.wrapping_add(len);
                            }
                            Input::Ping(seq) => send(socket, vec![session.pong(seq)?], &config).await?,
                            Input::Pong(_) => {}
                            Input::Dtmf(digits) => deliver(WebexInput::Dtmf(digits))?,
                            Input::DtmfStart => deliver(WebexInput::DtmfStarted)?,
                            Input::NoInput => deliver(WebexInput::NoInput)?,
                            Input::Custom { name, parameters } => {
                                deliver(WebexInput::Custom { name, parameters })?
                            }
                            Input::End => {
                                let _ = deliver(WebexInput::Ended);
                                return Ok(());
                            }
                            Input::Start => return Err(WebexError::State),
                        }
                    }
                    Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => {}
                    Some(Ok(WsMessage::Close(_))) | None => return Ok(()),
                    // Webex carries media as Base64 JSON; binary frames are invalid.
                    _ => return Err(WebexError::Protocol),
                }
            }
        }
    }
}

/// Finish a pending completion once its audio has arrived.
async fn settle<S>(
    socket: &mut WebSocketStream<S>,
    session: &mut Session,
    audio: &mut AgentAudio,
    pending: &mut Option<PendingCompletion>,
    route: &crate::adapter::Route,
    config: &crate::WebexByovaConfig,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    route
        .received
        .store(audio.received, std::sync::atomic::Ordering::Release);
    if pending
        .as_ref()
        .is_some_and(|pending| audio.received >= pending.position)
    {
        if let Some(pending) = pending.take() {
            let result = match finish_reply(session) {
                Ok(messages) => send(socket, messages, config).await,
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

async fn command<S>(
    request: CommandRequest,
    socket: &mut WebSocketStream<S>,
    session: &mut Session,
    audio: &mut AgentAudio,
    marks: &mut Marks,
    pending: &mut Option<PendingCompletion>,
    config: &crate::WebexByovaConfig,
) -> Result<Step>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let CommandRequest { command, reply } = request;
    let mut step = Step::Continue;
    let result = match command {
        Command::CompleteReply(boundary) => {
            let position = audio.position(boundary);
            if audio.received < position {
                *pending = Some(PendingCompletion {
                    position,
                    reply,
                    deadline: Instant::now() + config.reply_boundary_timeout,
                });
                return Ok(Step::Continue);
            }
            match finish_reply(session) {
                Ok(messages) => send(socket, messages, config).await,
                Err(error) => Err(error),
            }
        }
        Command::Interrupt(boundary) => {
            audio.fence(boundary);
            match session.interrupt() {
                Ok(messages) => send(socket, messages, config).await,
                Err(error) => Err(error.into()),
            }
        }
        Command::Event(event, transcript) => {
            match session.event(event.wire_name(), transcript.as_deref()) {
                Ok(message) => send(socket, vec![message], config).await,
                Err(error) => Err(error.into()),
            }
        }
        Command::Terminal { handoff } => match session.terminal(handoff) {
            Ok(messages) => {
                step = Step::Finished;
                send(socket, messages, config).await
            }
            Err(error) => Err(error.into()),
        },
        Command::FollowMarks(source) => marks.follow(source),
        Command::CompleteWithMetadata { name, metadata } => {
            match session.complete_with_metadata(&name, metadata) {
                Ok(messages) => {
                    step = Step::Finished;
                    send(socket, messages, config).await
                }
                Err(error) => Err(error.into()),
            }
        }
    };
    let fatal = matches!(result, Err(WebexError::Closed | WebexError::Timeout));
    let _ = reply.send(result);
    if fatal {
        return Err(WebexError::Closed);
    }
    Ok(step)
}

#[cfg(test)]
mod tests {
    use crate::adapter::{AgentAudio, ReplyBoundary};

    #[test]
    fn interruption_fence_discards_only_audio_before_the_boundary() {
        let mut audio = AgentAudio::default();
        assert_eq!(audio.accept(&[1; 100]).len(), 100);
        audio.fence(ReplyBoundary::At(250));
        assert!(audio.accept(&[2; 100]).is_empty());
        assert_eq!(audio.accept(&[3; 100]), &[3; 50][..]);
        assert_eq!(audio.accept(&[4; 10]).len(), 10);
        assert_eq!(audio.received, 310);
        audio.fence(ReplyBoundary::At(5));
        assert_eq!(audio.accept(&[5; 1]).len(), 1);
    }
}
