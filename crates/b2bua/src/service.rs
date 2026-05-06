use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dashmap::DashMap;
use rvoip_session_core::types::IncomingCallInfo;
use rvoip_session_core::{
    BridgeHandle, CallState, Config as SessionConfig, Event, EventReceiver, SessionId,
    UnifiedCoordinator,
};
use tokio::sync::broadcast;
use tokio::time::{sleep, Instant};
use tracing::{debug, info, warn};

use crate::error::{B2buaError, Result};
use crate::types::{
    B2buaCallHandle, B2buaCallId, B2buaCallSnapshot, B2buaCallStatus, B2buaEvent,
    B2buaEventReceiver, B2buaLeg, BridgeId, LegRole, RejectDecision, RouteDecision, RouteRequest,
};

/// Routing hook for inbound B2BUA calls.
///
/// Implement this trait in higher layers such as contact-center, voice-ai, or
/// a future CPaaS adapter. The B2BUA service remains responsible for the SIP
/// leg orchestration once a route decision is returned.
#[async_trait]
pub trait Router: Send + Sync + 'static {
    /// Select what should happen to an inbound call.
    async fn route(&self, request: RouteRequest) -> Result<RouteDecision>;
}

/// Router that always returns the same decision.
#[derive(Debug, Clone)]
pub struct StaticRouter {
    decision: RouteDecision,
}

impl StaticRouter {
    /// Always dial the supplied target.
    pub fn dial(target: impl Into<String>) -> Self {
        Self {
            decision: RouteDecision::dial(target),
        }
    }

    /// Always return the supplied route decision.
    pub fn new(decision: RouteDecision) -> Self {
        Self { decision }
    }

    /// Borrow the configured decision.
    pub fn decision(&self) -> &RouteDecision {
        &self.decision
    }
}

#[async_trait]
impl Router for StaticRouter {
    async fn route(&self, _request: RouteRequest) -> Result<RouteDecision> {
        Ok(self.decision.clone())
    }
}

/// Runtime configuration for the B2BUA layer.
#[derive(Debug, Clone)]
pub struct B2buaConfig {
    /// Local SIP URI used as the default From URI on outbound legs.
    pub local_uri: String,
    /// Timeout while waiting for an outbound leg to answer.
    pub outbound_answer_timeout: Duration,
    /// Timeout while waiting for a leg to reach `CallState::Active`.
    pub active_state_timeout: Duration,
    /// Capacity of the B2BUA broadcast event channel.
    pub event_channel_capacity: usize,
    /// SIP response used when routing fails before a route decision exists.
    pub route_error_reject: RejectDecision,
}

impl B2buaConfig {
    /// Build B2BUA config from a `session-core` config.
    pub fn from_session_config(config: &SessionConfig) -> Self {
        Self::new(config.local_uri.clone())
    }

    /// Build B2BUA config with practical defaults.
    pub fn new(local_uri: impl Into<String>) -> Self {
        Self {
            local_uri: local_uri.into(),
            outbound_answer_timeout: Duration::from_secs(30),
            active_state_timeout: Duration::from_secs(5),
            event_channel_capacity: 256,
            route_error_reject: RejectDecision::new(500, "Routing failed"),
        }
    }
}

struct B2buaInner {
    coordinator: Arc<UnifiedCoordinator>,
    config: B2buaConfig,
    events_tx: broadcast::Sender<B2buaEvent>,
    calls: DashMap<B2buaCallId, B2buaCallSnapshot>,
}

/// B2BUA service built on top of `session-core::UnifiedCoordinator`.
#[derive(Clone)]
pub struct B2buaService {
    inner: Arc<B2buaInner>,
}

impl B2buaService {
    /// Create a new `UnifiedCoordinator` and wrap it as a B2BUA service.
    pub async fn new(session_config: SessionConfig) -> Result<Self> {
        let config = B2buaConfig::from_session_config(&session_config);
        let coordinator = UnifiedCoordinator::new(session_config).await?;
        Ok(Self::from_coordinator(coordinator, config))
    }

    /// Wrap an existing coordinator.
    pub fn from_coordinator(coordinator: Arc<UnifiedCoordinator>, config: B2buaConfig) -> Self {
        let (events_tx, _) = broadcast::channel(config.event_channel_capacity.max(1));
        Self {
            inner: Arc::new(B2buaInner {
                coordinator,
                config,
                events_tx,
                calls: DashMap::new(),
            }),
        }
    }

    /// Return the underlying `session-core` coordinator.
    pub fn coordinator(&self) -> Arc<UnifiedCoordinator> {
        self.inner.coordinator.clone()
    }

    /// Subscribe to B2BUA-level events.
    pub fn events(&self) -> B2buaEventReceiver {
        self.inner.events_tx.subscribe()
    }

    /// Return the current snapshot for a B2BUA call.
    pub fn call(&self, id: &B2buaCallId) -> Option<B2buaCallSnapshot> {
        self.inner.calls.get(id).map(|entry| entry.clone())
    }

    /// Return all known call snapshots.
    pub fn calls(&self) -> Vec<B2buaCallSnapshot> {
        self.inner
            .calls
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Wait for the next inbound call, accept it into B2BUA ownership, and
    /// spawn a task that routes, dials, bridges, and tears it down.
    ///
    /// Returns `Ok(None)` when the underlying coordinator shuts down.
    pub async fn handle_next_call<R>(&self, router: Arc<R>) -> Result<Option<B2buaCallHandle>>
    where
        R: Router,
    {
        let Some(incoming) = self.inner.coordinator.get_incoming_call().await else {
            return Ok(None);
        };

        let call_id = B2buaCallId::new();
        let inbound = B2buaLeg {
            role: LegRole::Inbound,
            session_id: incoming.session_id.clone(),
            uri: incoming.from.clone(),
        };
        let snapshot = B2buaCallSnapshot::new(call_id.clone(), inbound.clone());
        self.inner.calls.insert(call_id.clone(), snapshot);

        self.emit(B2buaEvent::IncomingReceived {
            call_id: call_id.clone(),
            inbound: inbound.clone(),
            from: incoming.from.clone(),
            to: incoming.to.clone(),
        });

        let service = self.clone();
        let task_call_id = call_id.clone();
        tokio::spawn(async move {
            if let Err(err) = service
                .run_incoming_call(task_call_id.clone(), incoming, router)
                .await
            {
                warn!(%task_call_id, error = %err, "b2bua call failed");
                service.fail_call(&task_call_id, err.to_string()).await;
            }
        });

        Ok(Some(B2buaCallHandle {
            id: call_id,
            inbound,
        }))
    }

    /// Continuously serve inbound calls until the coordinator shuts down.
    ///
    /// Each accepted inbound call is handled in its own task.
    pub async fn serve<R>(&self, router: Arc<R>) -> Result<()>
    where
        R: Router,
    {
        while self.handle_next_call(router.clone()).await?.is_some() {}
        Ok(())
    }

    /// Spawn [`serve`](Self::serve) onto the current Tokio runtime.
    pub fn spawn_serve<R>(&self, router: Arc<R>) -> tokio::task::JoinHandle<Result<()>>
    where
        R: Router,
    {
        let service = self.clone();
        tokio::spawn(async move { service.serve(router).await })
    }

    async fn run_incoming_call<R>(
        &self,
        call_id: B2buaCallId,
        incoming: IncomingCallInfo,
        router: Arc<R>,
    ) -> Result<()>
    where
        R: Router,
    {
        let inbound_id = incoming.session_id.clone();
        let inbound_events = self
            .inner
            .coordinator
            .events_for_session(&inbound_id)
            .await?;

        self.update_call(&call_id, |snapshot| {
            snapshot.status = B2buaCallStatus::Routing;
        });

        let inbound = B2buaLeg {
            role: LegRole::Inbound,
            session_id: incoming.session_id.clone(),
            uri: incoming.from.clone(),
        };
        let route_request = RouteRequest {
            call_id: call_id.clone(),
            inbound,
            from: incoming.from.clone(),
            to: incoming.to.clone(),
            sip_call_id: incoming.call_id.clone(),
            p_asserted_identity: incoming.p_asserted_identity.clone(),
        };

        let decision = match router.route(route_request).await {
            Ok(decision) => decision,
            Err(err) => {
                let reject = self.inner.config.route_error_reject.clone();
                self.inner
                    .coordinator
                    .reject_call(&inbound_id, reject.status_code, &reject.reason)
                    .await?;
                self.emit(B2buaEvent::InboundRejected {
                    call_id: call_id.clone(),
                    status_code: reject.status_code,
                    reason: reject.reason.clone(),
                });
                self.update_call(&call_id, |snapshot| {
                    snapshot.status = B2buaCallStatus::Failed;
                    snapshot.reason = Some(err.to_string());
                });
                return Err(B2buaError::Route(err.to_string()));
            }
        };

        self.emit(B2buaEvent::RouteSelected {
            call_id: call_id.clone(),
            decision: decision.clone(),
        });

        match decision {
            RouteDecision::Dial { target, from } => {
                self.dial_and_bridge(call_id, inbound_id, inbound_events, target, from)
                    .await
            }
            RouteDecision::Reject(reject) => {
                self.inner
                    .coordinator
                    .reject_call(&inbound_id, reject.status_code, &reject.reason)
                    .await?;
                self.emit(B2buaEvent::InboundRejected {
                    call_id: call_id.clone(),
                    status_code: reject.status_code,
                    reason: reject.reason.clone(),
                });
                self.update_call(&call_id, |snapshot| {
                    snapshot.status = B2buaCallStatus::Rejected;
                    snapshot.reason = Some(reject.reason);
                });
                Ok(())
            }
            RouteDecision::Redirect(redirect) => {
                self.inner
                    .coordinator
                    .redirect_call(&inbound_id, redirect.status_code, redirect.contacts.clone())
                    .await?;
                self.emit(B2buaEvent::InboundRedirected {
                    call_id: call_id.clone(),
                    status_code: redirect.status_code,
                    contacts: redirect.contacts.clone(),
                });
                self.update_call(&call_id, |snapshot| {
                    snapshot.status = B2buaCallStatus::Redirected;
                    snapshot.reason = Some(format!("redirect {}", redirect.status_code));
                });
                Ok(())
            }
        }
    }

    async fn dial_and_bridge(
        &self,
        call_id: B2buaCallId,
        inbound_id: SessionId,
        mut inbound_events: EventReceiver,
        target: String,
        from: Option<String>,
    ) -> Result<()> {
        self.update_call(&call_id, |snapshot| {
            snapshot.status = B2buaCallStatus::Dialing;
        });

        let outbound_from = from.unwrap_or_else(|| self.inner.config.local_uri.clone());
        let outbound_id = match self
            .inner
            .coordinator
            .make_call(&outbound_from, &target)
            .await
        {
            Ok(outbound_id) => outbound_id,
            Err(err) => {
                self.reject_inbound_after_outbound_error(&call_id, &inbound_id, 480, "No route")
                    .await?;
                return Err(err.into());
            }
        };

        let outbound = B2buaLeg {
            role: LegRole::Outbound,
            session_id: outbound_id.clone(),
            uri: target.clone(),
        };
        self.update_call(&call_id, |snapshot| {
            snapshot.outbound = Some(outbound.clone());
        });
        self.emit(B2buaEvent::OutboundDialing {
            call_id: call_id.clone(),
            outbound,
            target,
        });

        let mut outbound_events = self
            .inner
            .coordinator
            .events_for_session(&outbound_id)
            .await?;

        let answer = self
            .wait_for_outbound_answer(
                &call_id,
                &inbound_id,
                &outbound_id,
                &mut inbound_events,
                &mut outbound_events,
            )
            .await;
        if let Err(err) = answer {
            let (status, reason) = outbound_error_to_reject(&err);
            self.reject_inbound_after_outbound_error(&call_id, &inbound_id, status, &reason)
                .await?;
            return Err(err);
        }

        self.update_call(&call_id, |snapshot| {
            snapshot.status = B2buaCallStatus::Answering;
        });
        self.inner.coordinator.accept_call(&inbound_id).await?;
        self.emit(B2buaEvent::InboundAccepted {
            call_id: call_id.clone(),
            inbound_session_id: inbound_id.clone(),
        });

        self.wait_for_active(&inbound_id).await?;
        self.wait_for_active(&outbound_id).await?;

        let bridge = self
            .inner
            .coordinator
            .bridge(&inbound_id, &outbound_id)
            .await?;
        let bridge_id = BridgeId::new();
        self.update_call(&call_id, |snapshot| {
            snapshot.status = B2buaCallStatus::Bridged;
            snapshot.bridge_id = Some(bridge_id.clone());
        });
        self.emit(B2buaEvent::BridgeEstablished {
            call_id: call_id.clone(),
            bridge_id: bridge_id.clone(),
            inbound_session_id: inbound_id.clone(),
            outbound_session_id: outbound_id.clone(),
        });
        info!(%call_id, %bridge_id, "b2bua bridge established");

        self.monitor_bridge(
            call_id,
            inbound_id,
            outbound_id,
            bridge,
            &mut inbound_events,
            &mut outbound_events,
        )
        .await
    }

    async fn wait_for_outbound_answer(
        &self,
        call_id: &B2buaCallId,
        inbound_id: &SessionId,
        outbound_id: &SessionId,
        inbound_events: &mut EventReceiver,
        outbound_events: &mut EventReceiver,
    ) -> Result<()> {
        let deadline = Instant::now() + self.inner.config.outbound_answer_timeout;
        loop {
            match self.inner.coordinator.get_state(outbound_id).await {
                Ok(CallState::Active) => {
                    self.emit(B2buaEvent::OutboundAnswered {
                        call_id: call_id.clone(),
                        outbound_session_id: outbound_id.clone(),
                        has_sdp: false,
                    });
                    return Ok(());
                }
                Ok(_) => {}
                Err(err) => return Err(err.into()),
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(B2buaError::OutboundAnswerTimeout {
                    session_id: outbound_id.clone(),
                    timeout: self.inner.config.outbound_answer_timeout,
                });
            }
            let slice = (deadline - now).min(Duration::from_millis(100));

            tokio::select! {
                inbound = inbound_events.next() => {
                    if let Some(reason) = self.observe_leg_event(
                        call_id,
                        LegRole::Inbound,
                        inbound_id,
                        inbound,
                    )? {
                        return Err(B2buaError::LegEndedBeforeBridge {
                            leg: LegRole::Inbound.as_str(),
                            session_id: inbound_id.clone(),
                            reason,
                        });
                    }
                }
                outbound = outbound_events.next() => {
                    match outbound {
                        Some(Event::CallProgress {
                            status_code,
                            reason,
                            ..
                        }) => {
                            self.emit(B2buaEvent::OutboundProgress {
                                call_id: call_id.clone(),
                                status_code,
                                reason,
                            });
                        }
                        Some(Event::CallAnswered { sdp, .. }) => {
                            self.emit(B2buaEvent::OutboundAnswered {
                                call_id: call_id.clone(),
                                outbound_session_id: outbound_id.clone(),
                                has_sdp: sdp.is_some(),
                            });
                            return Ok(());
                        }
                        Some(Event::CallFailed {
                            status_code,
                            reason,
                            ..
                        }) => {
                            return Err(B2buaError::OutboundFailed {
                                session_id: outbound_id.clone(),
                                status_code,
                                reason,
                            });
                        }
                        Some(Event::CallEnded { reason, .. }) => {
                            return Err(B2buaError::LegEndedBeforeBridge {
                                leg: LegRole::Outbound.as_str(),
                                session_id: outbound_id.clone(),
                                reason,
                            });
                        }
                        Some(Event::CallCancelled { .. }) => {
                            return Err(B2buaError::OutboundFailed {
                                session_id: outbound_id.clone(),
                                status_code: 487,
                                reason: "Request Terminated".to_string(),
                            });
                        }
                        Some(_) => {}
                        None => {
                            return Err(B2buaError::EventStreamClosed(outbound_id.clone()));
                        }
                    }
                }
                _ = sleep(slice) => {}
            }
        }
    }

    async fn wait_for_active(&self, session_id: &SessionId) -> Result<()> {
        let deadline = Instant::now() + self.inner.config.active_state_timeout;
        loop {
            match self.inner.coordinator.get_state(session_id).await {
                Ok(CallState::Active) => return Ok(()),
                Ok(_) => {}
                Err(err) => return Err(err.into()),
            }

            if Instant::now() >= deadline {
                return Err(B2buaError::ActiveStateTimeout {
                    session_id: session_id.clone(),
                    timeout: self.inner.config.active_state_timeout,
                });
            }
            sleep(Duration::from_millis(50)).await;
        }
    }

    async fn monitor_bridge(
        &self,
        call_id: B2buaCallId,
        inbound_id: SessionId,
        outbound_id: SessionId,
        bridge: BridgeHandle,
        inbound_events: &mut EventReceiver,
        outbound_events: &mut EventReceiver,
    ) -> Result<()> {
        let _bridge = bridge;
        let terminal = loop {
            tokio::select! {
                inbound = inbound_events.next() => {
                    match self.observe_leg_event(&call_id, LegRole::Inbound, &inbound_id, inbound)? {
                        Some(reason) => {
                            let _ = self.inner.coordinator.hangup(&outbound_id).await;
                            break reason;
                        }
                        None => {}
                    }
                }
                outbound = outbound_events.next() => {
                    match self.observe_leg_event(&call_id, LegRole::Outbound, &outbound_id, outbound)? {
                        Some(reason) => {
                            let _ = self.inner.coordinator.hangup(&inbound_id).await;
                            break reason;
                        }
                        None => {}
                    }
                }
            }
        };

        self.update_call(&call_id, |snapshot| {
            snapshot.status = B2buaCallStatus::Ended;
            snapshot.reason = Some(terminal.clone());
        });
        self.emit(B2buaEvent::CallEnded {
            call_id,
            reason: terminal,
        });
        Ok(())
    }

    fn observe_leg_event(
        &self,
        call_id: &B2buaCallId,
        leg: LegRole,
        session_id: &SessionId,
        event: Option<Event>,
    ) -> Result<Option<String>> {
        match event {
            Some(Event::DtmfReceived { digit, .. }) => {
                self.emit(B2buaEvent::DtmfReceived {
                    call_id: call_id.clone(),
                    leg,
                    digit,
                });
                Ok(None)
            }
            Some(Event::ReferReceived {
                refer_to,
                referred_by,
                replaces,
                ..
            }) => {
                self.emit(B2buaEvent::TransferRequested {
                    call_id: call_id.clone(),
                    leg,
                    refer_to,
                    referred_by,
                    replaces,
                });
                Ok(None)
            }
            Some(Event::CallEnded { reason, .. }) => {
                self.emit(B2buaEvent::LegEnded {
                    call_id: call_id.clone(),
                    leg,
                    session_id: session_id.clone(),
                    reason: reason.clone(),
                });
                Ok(Some(format!("{} leg ended: {}", leg.as_str(), reason)))
            }
            Some(Event::CallFailed {
                status_code,
                reason,
                ..
            }) => {
                let terminal = format!("{} leg failed: {} {}", leg.as_str(), status_code, reason);
                self.emit(B2buaEvent::LegEnded {
                    call_id: call_id.clone(),
                    leg,
                    session_id: session_id.clone(),
                    reason: terminal.clone(),
                });
                Ok(Some(terminal))
            }
            Some(Event::CallCancelled { .. }) => {
                let terminal = format!("{} leg cancelled", leg.as_str());
                self.emit(B2buaEvent::LegEnded {
                    call_id: call_id.clone(),
                    leg,
                    session_id: session_id.clone(),
                    reason: terminal.clone(),
                });
                Ok(Some(terminal))
            }
            Some(_) => Ok(None),
            None => Err(B2buaError::EventStreamClosed(session_id.clone())),
        }
    }

    async fn reject_inbound_after_outbound_error(
        &self,
        call_id: &B2buaCallId,
        inbound_id: &SessionId,
        status_code: u16,
        reason: &str,
    ) -> Result<()> {
        self.inner
            .coordinator
            .reject_call(inbound_id, status_code, reason)
            .await?;
        self.emit(B2buaEvent::InboundRejected {
            call_id: call_id.clone(),
            status_code,
            reason: reason.to_string(),
        });
        self.update_call(call_id, |snapshot| {
            snapshot.status = B2buaCallStatus::Failed;
            snapshot.reason = Some(reason.to_string());
        });
        Ok(())
    }

    async fn fail_call(&self, call_id: &B2buaCallId, reason: String) {
        let snapshot = self.call(call_id);
        self.update_call(call_id, |snapshot| {
            snapshot.status = B2buaCallStatus::Failed;
            snapshot.reason = Some(reason.clone());
        });
        self.emit(B2buaEvent::CallFailed {
            call_id: call_id.clone(),
            reason: reason.clone(),
        });

        if let Some(snapshot) = snapshot {
            let _ = self
                .inner
                .coordinator
                .hangup(&snapshot.inbound.session_id)
                .await;
            if let Some(outbound) = snapshot.outbound {
                let _ = self.inner.coordinator.hangup(&outbound.session_id).await;
            }
        }
    }

    fn update_call<F>(&self, call_id: &B2buaCallId, update: F)
    where
        F: FnOnce(&mut B2buaCallSnapshot),
    {
        if let Some(mut snapshot) = self.inner.calls.get_mut(call_id) {
            update(&mut snapshot);
        }
    }

    fn emit(&self, event: B2buaEvent) {
        if self.inner.events_tx.send(event).is_err() {
            debug!("b2bua event emitted with no subscribers");
        }
    }
}

fn outbound_error_to_reject(err: &B2buaError) -> (u16, String) {
    match err {
        B2buaError::OutboundFailed {
            status_code,
            reason,
            ..
        } if (400..=699).contains(status_code) => (*status_code, reason.clone()),
        B2buaError::OutboundAnswerTimeout { .. } => (480, "Temporarily Unavailable".to_string()),
        B2buaError::LegEndedBeforeBridge { reason, .. } => (480, reason.clone()),
        _ => (480, "Temporarily Unavailable".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn static_router_returns_configured_decision() {
        let router = StaticRouter::dial("sip:agent@example.com");
        let request = RouteRequest {
            call_id: B2buaCallId::new(),
            inbound: B2buaLeg {
                role: LegRole::Inbound,
                session_id: SessionId::new(),
                uri: "sip:caller@example.com".to_string(),
            },
            from: "sip:caller@example.com".to_string(),
            to: "sip:support@example.com".to_string(),
            sip_call_id: "sip-call-id".to_string(),
            p_asserted_identity: None,
        };

        assert_eq!(
            router.route(request).await.unwrap(),
            RouteDecision::dial("sip:agent@example.com")
        );
    }

    #[test]
    fn outbound_timeout_maps_to_480() {
        let err = B2buaError::OutboundAnswerTimeout {
            session_id: SessionId::new(),
            timeout: Duration::from_secs(1),
        };

        assert_eq!(
            outbound_error_to_reject(&err),
            (480, "Temporarily Unavailable".to_string())
        );
    }
}
