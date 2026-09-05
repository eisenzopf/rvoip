//! High-level caller-to-Vapi attachment and paired lifecycle supervision.

use std::fmt;
use std::sync::{Arc, Mutex as StdMutex, Weak};

use rvoip_core::adapter::{ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::events::Event;
use rvoip_core::ids::{BridgeId, ConnectionId};
use rvoip_core::Orchestrator;
use tokio::sync::{broadcast, watch};

use crate::adapter::VapiAdapter;
use crate::error::Result;
use crate::events::VapiEvent;
use crate::types::{VapiCallOptions, VapiPeerFailurePolicy};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VapiAgentOutcome {
    CallerEnded,
    AgentEnded,
    EventStreamClosed,
}

/// A bridged caller/Vapi pair.
///
/// The supervisor remains alive independently of this handle so dropping the
/// handle cannot silently disable symmetric teardown.
pub struct VapiAgentCall {
    orchestrator: Weak<Orchestrator>,
    adapter: Arc<VapiAdapter>,
    caller_connection_id: ConnectionId,
    vapi_connection_id: ConnectionId,
    bridge_id: BridgeId,
    events: broadcast::Sender<VapiEvent>,
    initial_events: StdMutex<Option<broadcast::Receiver<VapiEvent>>>,
    completion: watch::Receiver<Option<VapiAgentOutcome>>,
}

impl VapiAgentCall {
    pub fn caller_connection_id(&self) -> &ConnectionId {
        &self.caller_connection_id
    }

    pub fn vapi_connection_id(&self) -> &ConnectionId {
        &self.vapi_connection_id
    }

    pub fn bridge_id(&self) -> &BridgeId {
        &self.bridge_id
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<VapiEvent> {
        self.initial_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or_else(|| self.events.subscribe())
    }

    pub async fn say(
        &self,
        content: impl Into<String>,
        end_call_after_spoken: bool,
        interrupt_assistant: bool,
    ) -> Result<()> {
        self.adapter
            .say(
                &self.vapi_connection_id,
                content,
                end_call_after_spoken,
                interrupt_assistant,
            )
            .await
    }

    pub async fn add_message(
        &self,
        role: impl Into<String>,
        content: impl Into<String>,
        trigger_response: bool,
    ) -> Result<()> {
        self.adapter
            .add_message(&self.vapi_connection_id, role, content, trigger_response)
            .await
    }

    pub async fn mute_assistant(&self) -> Result<()> {
        self.adapter.mute_assistant(&self.vapi_connection_id).await
    }

    pub async fn unmute_assistant(&self) -> Result<()> {
        self.adapter
            .unmute_assistant(&self.vapi_connection_id)
            .await
    }

    /// End the Vapi leg. Under the default peer policy the supervisor then
    /// ends the paired caller leg as well.
    pub async fn end(&self) -> RvoipResult<()> {
        if let Some(orchestrator) = self.orchestrator.upgrade() {
            orchestrator
                .end_connection(self.vapi_connection_id.clone(), EndReason::Normal)
                .await
        } else {
            self.adapter
                .end(self.vapi_connection_id.clone(), EndReason::Normal)
                .await
        }
    }

    pub async fn wait(&mut self) -> VapiAgentOutcome {
        loop {
            if let Some(outcome) = *self.completion.borrow() {
                return outcome;
            }
            if self.completion.changed().await.is_err() {
                return VapiAgentOutcome::EventStreamClosed;
            }
        }
    }
}

impl fmt::Debug for VapiAgentCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VapiAgentCall")
            .field("caller_connection_id", &self.caller_connection_id)
            .field("vapi_connection_id", &self.vapi_connection_id)
            .field("bridge_id", &self.bridge_id)
            .finish()
    }
}

impl VapiAdapter {
    /// Originate a Vapi WebSocket call and bridge it to an existing caller.
    ///
    /// The adapter registers itself when no Vapi adapter is registered. A
    /// different pre-existing Vapi adapter is rejected to keep route ownership
    /// exact.
    pub async fn attach_agent(
        self: &Arc<Self>,
        orchestrator: &Arc<Orchestrator>,
        caller_connection_id: ConnectionId,
        options: VapiCallOptions,
    ) -> RvoipResult<VapiAgentCall> {
        options.validate().map_err(RvoipError::from)?;
        self.ensure_registered(orchestrator)?;

        let session_id = orchestrator
            .session_of(&caller_connection_id)
            .ok_or_else(|| RvoipError::ConnectionNotFound(caller_connection_id.clone()))?;
        let participant_id = {
            let session = orchestrator
                .session(&session_id)
                .ok_or_else(|| RvoipError::SessionNotFound(session_id.clone()))?;
            let session = session
                .read()
                .map_err(|_| RvoipError::InvalidState("session lock is poisoned"))?;
            session
                .connections
                .get(&caller_connection_id)
                .map(|connection| connection.participant_id.clone())
                .ok_or_else(|| RvoipError::ConnectionNotFound(caller_connection_id.clone()))?
        };

        let mut core_events = orchestrator.subscribe_events();
        let request = OriginateRequest::new(
            session_id,
            participant_id,
            "vapi.websocket",
            Direction::Outbound,
            options.audio_format.capabilities(),
        )
        .with_transport(Transport::Vapi)
        .with_context(options.clone());
        let handle = orchestrator.originate_connection(request).await?;
        let vapi_connection_id = handle.connection.id.clone();
        let initial_events = self.subscribe_call_events(&vapi_connection_id)?;
        let call_events = self.call_event_sender(&vapi_connection_id)?;

        let bridge_id = match orchestrator
            .bridge_connections(caller_connection_id.clone(), vapi_connection_id.clone())
            .await
        {
            Ok(bridge_id) => bridge_id,
            Err(error) => {
                let _ = orchestrator
                    .end_connection(
                        vapi_connection_id,
                        EndReason::Failed {
                            detail: "Vapi attachment bridge failed".into(),
                        },
                    )
                    .await;
                return Err(error);
            }
        };

        let (completion_tx, completion) = watch::channel(None);
        let supervisor_orchestrator = Arc::clone(orchestrator);
        let supervisor_adapter = Arc::clone(self);
        let supervisor_caller = caller_connection_id.clone();
        let supervisor_vapi = vapi_connection_id.clone();
        let supervisor_bridge = bridge_id.clone();
        tokio::spawn(async move {
            let outcome = supervise_pair(
                &supervisor_orchestrator,
                &supervisor_adapter,
                &mut core_events,
                &supervisor_caller,
                &supervisor_vapi,
                &supervisor_bridge,
                options.peer_failure_policy,
            )
            .await;
            let _ = completion_tx.send(Some(outcome));
        });

        Ok(VapiAgentCall {
            orchestrator: Arc::downgrade(orchestrator),
            adapter: Arc::clone(self),
            caller_connection_id,
            vapi_connection_id,
            bridge_id,
            events: call_events,
            initial_events: StdMutex::new(Some(initial_events)),
            completion,
        })
    }

    fn ensure_registered(self: &Arc<Self>, orchestrator: &Arc<Orchestrator>) -> RvoipResult<()> {
        let registration = match orchestrator.adapter(Transport::Vapi) {
            Ok(registered) => {
                let this: Arc<dyn ConnectionAdapter> =
                    Arc::clone(self) as Arc<dyn ConnectionAdapter>;
                if Arc::ptr_eq(&registered, &this) {
                    Ok(())
                } else {
                    Err(RvoipError::AdapterAlreadyRegistered(Transport::Vapi))
                }
            }
            Err(RvoipError::NoAdapterForTransport(Transport::Vapi)) => {
                orchestrator.register(Arc::clone(self) as Arc<dyn ConnectionAdapter>)
            }
            Err(error) => Err(error),
        };
        registration?;
        self.bind_orchestrator(orchestrator)
    }
}

async fn supervise_pair(
    orchestrator: &Arc<Orchestrator>,
    adapter: &Arc<VapiAdapter>,
    events: &mut broadcast::Receiver<Event>,
    caller: &ConnectionId,
    vapi: &ConnectionId,
    bridge: &BridgeId,
    peer_policy: VapiPeerFailurePolicy,
) -> VapiAgentOutcome {
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(broadcast::error::RecvError::Lagged(_)) => {
                if orchestrator.session_of(caller).is_none() {
                    let _ = orchestrator.unbridge_connections(bridge.clone()).await;
                    let _ = adapter.end(vapi.clone(), EndReason::BridgeTorn).await;
                    return VapiAgentOutcome::CallerEnded;
                }
                if !adapter.is_connection_live(vapi) {
                    let _ = orchestrator.unbridge_connections(bridge.clone()).await;
                    if peer_policy == VapiPeerFailurePolicy::EndCaller {
                        let _ = orchestrator
                            .end_connection(caller.clone(), EndReason::BridgeTorn)
                            .await;
                    }
                    return VapiAgentOutcome::AgentEnded;
                }
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => {
                let _ = orchestrator.unbridge_connections(bridge.clone()).await;
                let _ = adapter.end(vapi.clone(), EndReason::BridgeTorn).await;
                if peer_policy == VapiPeerFailurePolicy::EndCaller {
                    let _ = orchestrator
                        .end_connection(caller.clone(), EndReason::BridgeTorn)
                        .await;
                }
                return VapiAgentOutcome::EventStreamClosed;
            }
        };
        let terminal_connection = match event {
            Event::ConnectionEnded { connection_id, .. }
            | Event::ConnectionFailed { connection_id, .. } => Some(connection_id),
            _ => None,
        };
        let Some(terminal_connection) = terminal_connection else {
            continue;
        };
        if terminal_connection == *caller {
            let _ = orchestrator.unbridge_connections(bridge.clone()).await;
            let _ = orchestrator
                .end_connection(vapi.clone(), EndReason::BridgeTorn)
                .await;
            return VapiAgentOutcome::CallerEnded;
        }
        if terminal_connection == *vapi {
            let _ = orchestrator.unbridge_connections(bridge.clone()).await;
            if peer_policy == VapiPeerFailurePolicy::EndCaller {
                let _ = orchestrator
                    .end_connection(caller.clone(), EndReason::BridgeTorn)
                    .await;
            }
            return VapiAgentOutcome::AgentEnded;
        }
    }
}
