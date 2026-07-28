// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{future::Future, net, path::PathBuf, pin::Pin, sync::Arc, time::Duration};

use anyhow::Context;

use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use moq_native_ietf::quic::{self, Endpoint};
use tokio_util::{
    sync::CancellationToken,
    task::{task_tracker::TaskTrackerToken, TaskTracker},
};
use url::Url;

use crate::{
    metrics::GaugeGuard, AdmissionCloseContext, AdmissionCloseReason, AdmissionDecision,
    AdmissionRequest, AdmissionSessionId, AdmittedSession, Consumer, Coordinator,
    ListenerSecurityPolicy, Locals, Producer, RelayCapacity, RelayCapacityLimits, RelayIdentity,
    RemoteManager, RemoteManagerLimits, Session, SessionAdmission,
};

// A type alias for boxed future
type ServerFuture = Pin<
    Box<
        dyn Future<
                Output = (
                    anyhow::Result<quic::SessionConnection>,
                    quic::Server,
                    Arc<tokio::sync::Semaphore>,
                    Arc<tokio::sync::Semaphore>,
                ),
            > + Send,
    >,
>;

/// Configuration for the relay.
pub struct RelayConfig {
    /// Listen on this address
    pub bind: Option<net::SocketAddr>,

    /// Optional list of endpoints if provided, we won't use bind
    pub endpoints: Vec<Endpoint>,

    /// The TLS configuration.
    pub tls: moq_native_ietf::tls::Config,

    /// Directory to write qlog files (one per connection)
    pub qlog_dir: Option<PathBuf>,

    /// Directory to write mlog files (one per connection)
    pub mlog_dir: Option<PathBuf>,

    /// Forward all PUBLISH_NAMESPACE messages to the (optional) upstream URL.
    pub announce: Option<Url>,

    /// Our hostname which we advertise to other origins.
    /// We use QUIC, so the certificate must be valid for this address.
    pub node: Option<Url>,

    /// The coordinator for namespace/track registration and discovery.
    pub coordinator: Arc<dyn Coordinator>,

    /// Admission policy evaluated before coordinator or media state mutation.
    pub admission: Arc<dyn SessionAdmission>,

    /// Explicitly enables development-only policies and anonymous TLS peers.
    pub development: bool,

    /// Security role shared by this relay process's inbound listeners.
    /// Deploy separate relay processes when publisher and browser listener
    /// roles require different TLS postures.
    pub listener_security: ListenerSecurityPolicy,

    pub setup_timeout: Duration,
    pub admission_timeout: Duration,
    pub cleanup_timeout: Duration,
    /// Maximum time to await replay tombstoning and distributed policy-lease
    /// release after transport and session work have stopped.
    pub session_close_timeout: Duration,
    pub max_pending_admissions: usize,
    pub max_active_sessions: usize,
    pub token_revalidation_interval: Duration,

    /// Hierarchical limits for retained namespace, track, subscription, and
    /// track-status request state.
    pub capacity_limits: RelayCapacityLimits,

    /// Limits and idle eviction windows for retained upstream relay state.
    pub remote_limits: RemoteManagerLimits,

    /// Per-published-namespace track cache and pending request limits.
    pub tracks_limits: moq_transport::serve::TracksLimits,

    /// Process-shared transport request and retained-media limits.
    pub request_limits: moq_transport::session::RequestLimits,
}

/// MoQ Relay server.
pub struct Relay {
    quic_endpoints: Vec<Endpoint>,
    announce_url: Option<Url>,
    mlog_dir: Option<PathBuf>,
    locals: Locals,
    remotes: RemoteManager,
    coordinator: Arc<dyn Coordinator>,
    admission: Arc<dyn SessionAdmission>,
    listener_security: ListenerSecurityPolicy,
    setup_timeout: Duration,
    admission_timeout: Duration,
    cleanup_timeout: Duration,
    session_close_timeout: Duration,
    max_pending_admissions: usize,
    max_active_sessions: usize,
    production: bool,
    token_revalidation_interval: Duration,
    capacity: RelayCapacity,
    tracks_limits: moq_transport::serve::TracksLimits,
    request_capacity: moq_transport::session::RequestCapacity,
}

/// Cloneable aggregate diagnostics that can be retained while [`Relay::run`] owns the server.
#[derive(Clone)]
pub struct RelayDiagnostics {
    capacity: RelayCapacity,
    remotes: RemoteManager,
    request_capacity: moq_transport::session::RequestCapacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelayDiagnosticsSnapshot {
    pub capacity: crate::RelayCapacitySnapshot,
    pub remotes: crate::RemoteManagerSnapshot,
    pub retained_process_bytes: usize,
    pub max_retained_process_bytes: usize,
}

impl RelayDiagnostics {
    pub async fn snapshot(&self) -> RelayDiagnosticsSnapshot {
        let retention = self.request_capacity.retention_stats();
        RelayDiagnosticsSnapshot {
            capacity: self.capacity.snapshot(),
            remotes: self.remotes.snapshot().await,
            retained_process_bytes: retention.process_bytes,
            max_retained_process_bytes: retention.max_process_bytes,
        }
    }
}

fn listener_decision_is_valid(
    policy: ListenerSecurityPolicy,
    peer_identity: &moq_native_ietf::tls::PeerIdentity,
    setup_authorization: Option<&moq_transport::session::SetupAuthorization>,
    substrate: moq_transport::session::Transport,
    negotiated_protocol: &str,
    decision: &AdmissionDecision,
    production: bool,
) -> bool {
    if decision.claims.validate().is_err()
        || !listener_session_is_allowed(policy, substrate, negotiated_protocol, setup_authorization)
    {
        return false;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(u64::MAX, |duration| duration.as_secs());
    match policy {
        ListenerSecurityPolicy::MutualTlsPublisher
        | ListenerSecurityPolicy::MutualTlsRelaySubscriber => {
            peer_identity.is_authenticated()
                && decision_matches_listener_role(policy, decision, production, now)
        }
        ListenerSecurityPolicy::TokenSubscriber
        | ListenerSecurityPolicy::RawQuicTokenSubscriber => {
            decision_matches_listener_role(policy, decision, production, now)
        }
        ListenerSecurityPolicy::Development => {
            decision_matches_listener_role(policy, decision, production, now)
        }
    }
}

fn listener_session_is_allowed(
    policy: ListenerSecurityPolicy,
    substrate: moq_transport::session::Transport,
    negotiated_protocol: &str,
    setup_authorization: Option<&moq_transport::session::SetupAuthorization>,
) -> bool {
    match policy {
        ListenerSecurityPolicy::MutualTlsPublisher => {
            substrate == moq_transport::session::Transport::RawQuic
        }
        ListenerSecurityPolicy::MutualTlsRelaySubscriber => {
            substrate == moq_transport::session::Transport::RawQuic
                && negotiated_protocol.as_bytes() == moq_transport::setup::ALPN
                && setup_authorization.is_none()
        }
        ListenerSecurityPolicy::TokenSubscriber => {
            substrate == moq_transport::session::Transport::WebTransport
                && negotiated_protocol.as_bytes() == moq_transport::setup::ALPN
                && setup_authorization.is_some_and(|token| !token.is_empty())
        }
        ListenerSecurityPolicy::RawQuicTokenSubscriber => {
            substrate == moq_transport::session::Transport::RawQuic
                && negotiated_protocol.as_bytes() == moq_transport::setup::ALPN
                && setup_authorization.is_some_and(|token| !token.is_empty())
        }
        ListenerSecurityPolicy::Development => true,
    }
}

fn decision_matches_listener_role(
    policy: ListenerSecurityPolicy,
    decision: &AdmissionDecision,
    production: bool,
    now: u64,
) -> bool {
    match policy {
        ListenerSecurityPolicy::MutualTlsPublisher => {
            decision.principal.method == crate::AuthenticationMethod::MutualTls
                && decision.claims.publish
                && !decision.claims.subscribe
                && decision.claims.scope.is_some()
        }
        ListenerSecurityPolicy::MutualTlsRelaySubscriber => {
            decision.principal.method == crate::AuthenticationMethod::MutualTls
                && decision.claims.subscribe
                && !decision.claims.publish
                && decision.claims.scope.is_some()
                && decision.claims.expires_at_unix_seconds.is_none()
                && decision.claims.token_id.is_none()
        }
        ListenerSecurityPolicy::TokenSubscriber
        | ListenerSecurityPolicy::RawQuicTokenSubscriber => {
            let base = decision.principal.method == crate::AuthenticationMethod::SetupToken
                && decision.claims.subscribe
                && !decision.claims.publish;
            if !base || !production {
                return base;
            }
            decision
                .claims
                .scope
                .as_ref()
                .is_some_and(|scope| !scope.is_empty())
                && decision
                    .claims
                    .token_id
                    .as_ref()
                    .is_some_and(|token_id| !token_id.is_empty())
                && decision
                    .claims
                    .expires_at_unix_seconds
                    .is_some_and(|expiry| expiry > now)
        }
        ListenerSecurityPolicy::Development => {
            decision.principal.method == crate::AuthenticationMethod::Development
        }
    }
}

fn resolved_scope_is_valid(scope_id: &str) -> bool {
    !scope_id.is_empty()
        && scope_id.len() <= crate::AdmissionClaims::MAX_SCOPE_BYTES
        && !scope_id.chars().any(char::is_control)
}

fn should_log_admission_warning() -> bool {
    static WARNINGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = WARNINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    sequence < 4 || sequence.is_multiple_of(128)
}

async fn report_retention_metrics(
    capacity: moq_transport::session::RequestCapacity,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    loop {
        let stats = capacity.retention_stats();
        metrics::gauge!("moq_relay_retained_bytes", "scope" => "process")
            .set(stats.process_bytes as f64);
        metrics::gauge!("moq_relay_retained_bytes_limit", "scope" => "process")
            .set(stats.max_process_bytes as f64);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            _ = shutdown.cancelled() => return Ok(()),
        }
    }
}

async fn close_and_wait(
    session: &web_transport::Session,
    code: u32,
    reason: &str,
    timeout: Duration,
) {
    session.close(code, reason);
    if tokio::time::timeout(timeout, session.closed())
        .await
        .is_err()
    {
        tracing::debug!(code, "timed out waiting for connection cleanup");
    }
}

async fn monitor_token_lease(
    admitted: &AdmittedSession,
    interval: Duration,
    validation_timeout: Duration,
) -> Result<(), crate::AdmissionError> {
    monitor_token_lease_with_clock(
        admitted,
        interval,
        validation_timeout,
        || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(u64::MAX, |duration| duration.as_secs())
        },
        tokio::time::sleep,
    )
    .await
}

async fn monitor_token_lease_with_clock<Now, Sleep, SleepFuture>(
    admitted: &AdmittedSession,
    interval: Duration,
    validation_timeout: Duration,
    mut now: Now,
    mut sleep: Sleep,
) -> Result<(), crate::AdmissionError>
where
    Now: FnMut() -> u64,
    Sleep: FnMut(Duration) -> SleepFuture,
    SleepFuture: Future<Output = ()>,
{
    loop {
        let expiry = admitted
            .decision()
            .claims
            .expires_at_unix_seconds
            .ok_or(crate::AdmissionError::PolicyDenied)?;
        let before_sleep = now();
        if expiry <= before_sleep {
            return Err(crate::AdmissionError::PolicyDenied);
        }
        let until_expiry = Duration::from_secs(expiry - before_sleep);
        sleep(interval.min(until_expiry)).await;

        // Expiry is an independent hard deadline. Re-check it immediately
        // after sleeping so an exact-boundary session cannot remain active
        // for an additional admission/revalidation timeout.
        if expiry <= now() {
            return Err(crate::AdmissionError::PolicyDenied);
        }
        tokio::time::timeout(validation_timeout, admitted.revalidate(now()))
            .await
            .map_err(|_| crate::AdmissionError::PolicyDenied)??;
    }
}

async fn revalidate_token_before_activation(
    admitted: &AdmittedSession,
    validation_timeout: Duration,
    now: u64,
) -> Result<(), crate::AdmissionError> {
    if admitted
        .decision()
        .claims
        .expires_at_unix_seconds
        .is_none_or(|expiry| expiry <= now)
    {
        return Err(crate::AdmissionError::PolicyDenied);
    }
    tokio::time::timeout(validation_timeout, admitted.revalidate(now))
        .await
        .map_err(|_| crate::AdmissionError::PolicyDenied)??;
    Ok(())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(u64::MAX, |duration| duration.as_secs())
}

#[derive(Clone, Copy, Debug)]
struct AdmittedClose {
    reason: AdmissionCloseReason,
    transport_code: u32,
    transport_reason: &'static str,
}

impl AdmittedClose {
    const fn activation(reason: &'static str) -> Self {
        Self {
            reason: AdmissionCloseReason::ActivationFailed,
            transport_code: moq_transport::session::SessionTerminationCode::Unauthorized as u32,
            transport_reason: reason,
        }
    }

    const fn shutdown() -> Self {
        Self {
            reason: AdmissionCloseReason::RelayShutdown,
            transport_code: 0,
            transport_reason: "relay draining",
        }
    }

    const fn cancelled() -> Self {
        Self {
            reason: AdmissionCloseReason::LocalClosed,
            transport_code: moq_transport::session::SessionTerminationCode::InternalError as u32,
            transport_reason: "admitted session task cancelled",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdmissionFinalization {
    Closed,
    CloseError,
    CloseTimeout,
}

async fn close_admission_lease(
    admitted: &mut AdmittedSession,
    reason: AdmissionCloseReason,
    close_timeout: Duration,
) -> AdmissionFinalization {
    let context = AdmissionCloseContext {
        reason,
        ended_at_unix_seconds: unix_now(),
    };
    match tokio::time::timeout(close_timeout, admitted.close(context)).await {
        Ok(Ok(())) => {
            metrics::counter!(
                "moq_relay_admission_close_total",
                "outcome" => "success",
                "reason" => reason.as_str()
            )
            .increment(1);
            AdmissionFinalization::Closed
        }
        Ok(Err(error)) => {
            tracing::error!(%error, reason = reason.as_str(), "admission session close failed closed");
            metrics::counter!(
                "moq_relay_admission_close_total",
                "outcome" => "error",
                "reason" => reason.as_str()
            )
            .increment(1);
            metrics::counter!(
                "moq_relay_connection_errors_total",
                "stage" => "admission_close_error"
            )
            .increment(1);
            AdmissionFinalization::CloseError
        }
        Err(_) => {
            tracing::error!(
                reason = reason.as_str(),
                "admission session close timed out; backing state must remain fail-closed"
            );
            metrics::counter!(
                "moq_relay_admission_close_total",
                "outcome" => "timeout",
                "reason" => reason.as_str()
            )
            .increment(1);
            metrics::counter!(
                "moq_relay_connection_errors_total",
                "stage" => "admission_close_timeout"
            )
            .increment(1);
            AdmissionFinalization::CloseTimeout
        }
    }
}

async fn finalize_admitted_session(
    raw_conn: &web_transport::Session,
    admitted: &mut AdmittedSession,
    close: AdmittedClose,
    cleanup_timeout: Duration,
    session_close_timeout: Duration,
) -> AdmissionFinalization {
    close_and_wait(
        raw_conn,
        close.transport_code,
        close.transport_reason,
        cleanup_timeout,
    )
    .await;
    close_admission_lease(admitted, close.reason, session_close_timeout).await
}

struct AdmissionLifecycleResources {
    raw_conn: web_transport::Session,
    connection_close_metric: Arc<std::sync::atomic::AtomicBool>,
    pending_admission_permit: Option<tokio::sync::OwnedSemaphorePermit>,
    active_session_permit: tokio::sync::OwnedSemaphorePermit,
    cleanup_timeout: Duration,
    session_close_timeout: Duration,
}

struct AdmittedFinalizationResources {
    admitted: AdmittedSession,
    lifecycle: AdmissionLifecycleResources,
}

async fn finalize_admitted_resources(
    mut resources: AdmittedFinalizationResources,
    close: AdmittedClose,
) -> AdmissionFinalization {
    let finalization = finalize_admitted_session(
        &resources.lifecycle.raw_conn,
        &mut resources.admitted,
        close,
        resources.lifecycle.cleanup_timeout,
        resources.lifecycle.session_close_timeout,
    )
    .await;
    // Both local permits remain owned by `resources` until the awaited close
    // attempt completes. Durable policy state must remain fail-closed if that
    // attempt reports an error or reaches its deadline.
    drop(resources.lifecycle.pending_admission_permit.take());
    drop(resources.lifecycle.active_session_permit);
    record_connection_closed(&resources.lifecycle.connection_close_metric);
    tracing::debug!(
        ?finalization,
        reason = close.reason.as_str(),
        "admitted session teardown complete"
    );
    finalization
}

fn record_connection_closed(recorded: &std::sync::atomic::AtomicBool) {
    if !recorded.swap(true, std::sync::atomic::Ordering::AcqRel) {
        metrics::counter!("moq_relay_connections_closed_total").increment(1);
    }
}

#[derive(Clone)]
struct AdmissionReaper {
    tasks: TaskTracker,
}

impl AdmissionReaper {
    fn new() -> Self {
        Self {
            tasks: TaskTracker::new(),
        }
    }

    fn handoff_token(&self) -> TaskTrackerToken {
        self.tasks.token()
    }

    fn spawn_finalizer(
        &self,
        resources: AdmittedFinalizationResources,
        handoff: TaskTrackerToken,
        close: AdmittedClose,
    ) -> tokio::task::JoinHandle<AdmissionFinalization> {
        self.tasks.spawn(async move {
            let _handoff = handoff;
            finalize_admitted_resources(resources, close).await
        })
    }

    fn reap_late_admission(
        &self,
        attempt: tokio::task::JoinHandle<Result<AdmittedSessionGuard, crate::AdmissionError>>,
    ) {
        self.tasks.spawn(async move {
            match attempt.await {
                Ok(Ok(guard)) => {
                    guard
                        .finalize(AdmittedClose::activation(
                            "admission completed after its deadline",
                        ))
                        .await;
                }
                Ok(Err(error)) => {
                    tracing::debug!(%error, "timed-out admission eventually denied");
                }
                Err(error) => {
                    tracing::error!(%error, "timed-out admission task failed");
                    metrics::counter!(
                        "moq_relay_connection_errors_total",
                        "stage" => "admission_task"
                    )
                    .increment(1);
                }
            }
        });
    }

    async fn drain(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

struct AdmittedSessionGuard {
    resources: Option<AdmittedFinalizationResources>,
    reaper: AdmissionReaper,
    handoff: Option<TaskTrackerToken>,
}

impl AdmittedSessionGuard {
    fn new(resources: AdmittedFinalizationResources, reaper: AdmissionReaper) -> Self {
        let handoff = reaper.handoff_token();
        Self {
            resources: Some(resources),
            reaper,
            handoff: Some(handoff),
        }
    }

    fn admitted(&self) -> Option<&AdmittedSession> {
        self.resources.as_ref().map(|resources| &resources.admitted)
    }

    fn release_pending_admission(&mut self) {
        if let Some(resources) = self.resources.as_mut() {
            drop(resources.lifecycle.pending_admission_permit.take());
        }
    }

    async fn finalize(mut self, close: AdmittedClose) -> AdmissionFinalization {
        match (self.resources.take(), self.handoff.take()) {
            (Some(resources), Some(handoff)) => {
                match self.reaper.spawn_finalizer(resources, handoff, close).await {
                    Ok(finalization) => finalization,
                    Err(error) => {
                        tracing::error!(%error, "admitted-session finalizer task failed");
                        AdmissionFinalization::CloseError
                    }
                }
            }
            _ => {
                tracing::error!("admitted-session guard lost finalization ownership");
                AdmissionFinalization::CloseError
            }
        }
    }
}

impl Drop for AdmittedSessionGuard {
    fn drop(&mut self) {
        if let (Some(resources), Some(handoff)) = (self.resources.take(), self.handoff.take()) {
            drop(
                self.reaper
                    .spawn_finalizer(resources, handoff, AdmittedClose::cancelled()),
            );
        }
    }
}

struct OwnedAdmissionRequest {
    session_id: AdmissionSessionId,
    peer_identity: moq_native_ietf::tls::PeerIdentity,
    target: moq_transport::session::SessionTarget,
    substrate: moq_transport::session::Transport,
    negotiated_protocol: &'static str,
    setup_authorization: Option<moq_transport::session::SetupAuthorization>,
}

struct AdmissionAttempt {
    admission: Arc<dyn SessionAdmission>,
    request: OwnedAdmissionRequest,
    resources: AdmissionLifecycleResources,
    reaper: AdmissionReaper,
}

impl AdmissionAttempt {
    fn spawn(self) -> tokio::task::JoinHandle<Result<AdmittedSessionGuard, crate::AdmissionError>> {
        let tasks = self.reaper.tasks.clone();
        tasks.spawn(async move {
            let admitted = self
                .admission
                .admit_session(AdmissionRequest {
                    session_id: &self.request.session_id,
                    peer_identity: &self.request.peer_identity,
                    target: &self.request.target,
                    substrate: self.request.substrate,
                    negotiated_protocol: self.request.negotiated_protocol,
                    setup_authorization: self.request.setup_authorization.as_ref(),
                })
                .await?;
            Ok(AdmittedSessionGuard::new(
                AdmittedFinalizationResources {
                    admitted,
                    lifecycle: self.resources,
                },
                self.reaper,
            ))
        })
    }
}

struct AdmittedConnectionContext {
    raw_conn: web_transport::Session,
    peer_identity: moq_native_ietf::tls::PeerIdentity,
    listener_security: ListenerSecurityPolicy,
    production: bool,
    coordinator: Arc<dyn Coordinator>,
    locals: Locals,
    remotes: RemoteManager,
    forward: Option<Producer>,
    capacity: RelayCapacity,
    tracks_limits: moq_transport::serve::TracksLimits,
    admission_timeout: Duration,
    cleanup_timeout: Duration,
    token_revalidation_interval: Duration,
    shutdown: CancellationToken,
}

struct ActiveSessionContext<'a> {
    raw_conn: &'a web_transport::Session,
    admitted: &'a AdmittedSession,
    production_token: bool,
    token_revalidation_interval: Duration,
    admission_timeout: Duration,
    cleanup_timeout: Duration,
    shutdown: &'a CancellationToken,
}

async fn run_active_session(session: Session, context: ActiveSessionContext<'_>) -> AdmittedClose {
    let session_run = session.run();
    tokio::pin!(session_run);
    let mut completed = false;

    let close = if context.production_token {
        tokio::select! {
            biased;
            _ = context.shutdown.cancelled() => AdmittedClose::shutdown(),
            result = &mut session_run => {
                completed = true;
                close_for_session_result(result)
            }
            lease = monitor_token_lease(
                context.admitted,
                context.token_revalidation_interval,
                context.admission_timeout,
            ) => {
                tracing::warn!(error = ?lease.err(), "token admission lease expired or was revoked");
                metrics::counter!(
                    "moq_relay_connection_errors_total",
                    "stage" => "admission_revalidation"
                )
                .increment(1);
                AdmittedClose {
                    reason: AdmissionCloseReason::AdmissionRevalidationFailed,
                    transport_code: moq_transport::session::SessionTerminationCode::Unauthorized
                        .as_u32(),
                    transport_reason: "token admission lease expired",
                }
            }
        }
    } else {
        tokio::select! {
            biased;
            _ = context.shutdown.cancelled() => AdmittedClose::shutdown(),
            result = &mut session_run => {
                completed = true;
                close_for_session_result(result)
            }
        }
    };

    if !completed {
        context
            .raw_conn
            .close(close.transport_code, close.transport_reason);
        if tokio::time::timeout(context.cleanup_timeout, &mut session_run)
            .await
            .is_err()
        {
            tracing::debug!(
                reason = close.reason.as_str(),
                "timed out waiting for session tasks during admitted teardown"
            );
        }
    }

    close
}

fn close_for_session_result(
    result: Result<(), moq_transport::session::SessionError>,
) -> AdmittedClose {
    match result {
        Ok(()) => {
            tracing::debug!("MoQ session ended cleanly");
            AdmittedClose {
                reason: AdmissionCloseReason::PeerClosed,
                transport_code: 0,
                transport_reason: "session complete",
            }
        }
        Err(error) if error.is_graceful_close() => {
            tracing::debug!("MoQ session closed gracefully");
            AdmittedClose {
                reason: AdmissionCloseReason::PeerClosed,
                transport_code: 0,
                transport_reason: "peer closed",
            }
        }
        Err(error) => {
            tracing::warn!(%error, "MoQ session error");
            metrics::counter!(
                "moq_relay_connection_errors_total",
                "stage" => "session_run"
            )
            .increment(1);
            AdmittedClose {
                reason: AdmissionCloseReason::ProtocolError,
                transport_code: moq_transport::session::SessionTerminationCode::ProtocolViolation
                    .as_u32(),
                transport_reason: "session protocol error",
            }
        }
    }
}

async fn run_admitted_connection(
    mut admitted: AdmittedSessionGuard,
    mut moq_session: moq_transport::session::Session,
    publisher: Option<moq_transport::session::Publisher>,
    subscriber: Option<moq_transport::session::Subscriber>,
    context: AdmittedConnectionContext,
) {
    let Some(decision) = admitted
        .admitted()
        .map(|admitted| admitted.decision().clone())
    else {
        admitted.finalize(AdmittedClose::cancelled()).await;
        return;
    };

    let close = async {
        if !listener_decision_is_valid(
            context.listener_security,
            &context.peer_identity,
            moq_session.peer_setup_authorization(),
            moq_session.transport(),
            moq_session.negotiated_transport().protocol,
            &decision,
            context.production,
        ) {
            if should_log_admission_warning() {
                tracing::warn!(
                    target = %moq_session.target().redacted_for_logging(),
                    "admission decision violates listener security policy (warnings sampled)"
                );
            }
            metrics::counter!(
                "moq_relay_connection_errors_total",
                "stage" => "admission_claims"
            )
            .increment(1);
            return AdmittedClose::activation("session admission denied");
        }
        moq_session.clear_peer_setup_authorization();

        let scope_info = match tokio::time::timeout(
            context.admission_timeout,
            context
                .coordinator
                .resolve_admitted_scope(&decision, moq_session.connection_path()),
        )
        .await
        {
            Ok(Ok(info)) => info,
            Ok(Err(error)) => {
                tracing::warn!(
                    connection_target = %moq_session.target().redacted_for_logging(),
                    query_present = moq_session.target().query().is_some(),
                    %error,
                    "scope resolution failed, rejecting session"
                );
                metrics::counter!(
                    "moq_relay_connection_errors_total",
                    "stage" => "scope_resolve"
                )
                .increment(1);
                return AdmittedClose::activation("scope resolution failed");
            }
            Err(_) => {
                tracing::warn!("scope resolution timed out");
                metrics::counter!(
                    "moq_relay_connection_errors_total",
                    "stage" => "scope_timeout"
                )
                .increment(1);
                return AdmittedClose::activation("scope resolution timeout");
            }
        };

        if decision.claims.scope.is_some() && scope_info.is_none() {
            tracing::warn!(
                connection_target = %moq_session.target().redacted_for_logging(),
                query_present = moq_session.target().query().is_some(),
                "scoped admission did not resolve to a coordinator scope"
            );
            metrics::counter!(
                "moq_relay_connection_errors_total",
                "stage" => "scope_missing"
            )
            .increment(1);
            return AdmittedClose::activation("admitted scope not found");
        }
        if scope_info
            .as_ref()
            .is_some_and(|scope| !resolved_scope_is_valid(&scope.scope_id))
        {
            tracing::warn!("coordinator returned an invalid scope identity");
            metrics::counter!(
                "moq_relay_connection_errors_total",
                "stage" => "scope_invalid"
            )
            .increment(1);
            return AdmittedClose::activation("invalid resolved scope");
        }

        let production_token = matches!(
            context.listener_security,
            ListenerSecurityPolicy::TokenSubscriber
                | ListenerSecurityPolicy::RawQuicTokenSubscriber
        ) && context.production;
        if production_token {
            let now = unix_now();
            let validation = match admitted.admitted() {
                Some(admitted) => {
                    revalidate_token_before_activation(admitted, context.admission_timeout, now)
                        .await
                }
                None => Err(crate::AdmissionError::PolicyDenied),
            };
            if let Err(error) = validation {
                tracing::warn!(%error, "token lease expired or was revoked before activation");
                metrics::counter!(
                    "moq_relay_connection_errors_total",
                    "stage" => "admission_activation"
                )
                .increment(1);
                return AdmittedClose {
                    reason: AdmissionCloseReason::AdmissionRevalidationFailed,
                    transport_code: moq_transport::session::SessionTerminationCode::Unauthorized
                        .as_u32(),
                    transport_reason: "token admission expired before activation",
                };
            }
        }

        admitted.release_pending_admission();
        let scope_id = scope_info.as_ref().map(|scope| scope.scope_id.clone());
        let identity = RelayIdentity::admitted(&decision, scope_id);
        let can_publish = decision.claims.publish
            && scope_info
                .as_ref()
                .is_none_or(|scope| scope.permissions.can_publish());
        let can_subscribe = decision.claims.subscribe
            && scope_info
                .as_ref()
                .is_none_or(|scope| scope.permissions.can_subscribe());

        if let Some(ref info) = scope_info {
            tracing::debug!(
                connection_target = %moq_session.target().redacted_for_logging(),
                query_present = moq_session.target().query().is_some(),
                scope_bound = !info.scope_id.is_empty(),
                permissions = ?info.permissions,
                "scope resolved"
            );
        }

        let (producer, reject_subscribes) = if can_subscribe {
            (
                publisher.map(|publisher| {
                    Producer::new_admitted(
                        publisher,
                        context.locals.clone(),
                        context.remotes.clone(),
                        identity.clone(),
                        context.capacity.clone(),
                    )
                }),
                None,
            )
        } else {
            (None, publisher)
        };
        let (consumer, reject_publishes) = if can_publish {
            (
                subscriber.map(|subscriber| {
                    Consumer::new_admitted(
                        subscriber,
                        context.locals.clone(),
                        context.coordinator.clone(),
                        context.forward.clone(),
                        identity,
                        context.capacity.clone(),
                        context.tracks_limits,
                    )
                }),
                None,
            )
        } else {
            (None, subscriber)
        };

        let session = Session {
            session: moq_session,
            producer,
            consumer,
            reject_publishes,
            reject_subscribes,
        };
        match admitted.admitted() {
            Some(admitted) => {
                run_active_session(
                    session,
                    ActiveSessionContext {
                        raw_conn: &context.raw_conn,
                        admitted,
                        production_token,
                        token_revalidation_interval: context.token_revalidation_interval,
                        admission_timeout: context.admission_timeout,
                        cleanup_timeout: context.cleanup_timeout,
                        shutdown: &context.shutdown,
                    },
                )
                .await
            }
            None => AdmittedClose::cancelled(),
        }
    }
    .await;

    admitted.finalize(close).await;
}

impl Relay {
    /// Construct a relay with an isolated local publication registry.
    ///
    /// This remains the safe default for one-listener deployments. Embedded
    /// deployments with role-separated publisher and subscriber listeners can
    /// use [`Self::new_with_locals`] to route both listeners through the same
    /// bounded in-process registry.
    pub fn new(config: RelayConfig) -> anyhow::Result<Self> {
        Self::new_with_locals(config, Locals::new())
    }

    /// Construct a relay using an application-owned local publication
    /// registry.
    ///
    /// Sharing [`Locals`] shares media routing only; it does not share or
    /// weaken listener admission, TLS policy, capacity accounting, or session
    /// lifecycles. Callers must still configure each listener with the
    /// appropriate production security role.
    pub fn new_with_locals(config: RelayConfig, locals: Locals) -> anyhow::Result<Self> {
        if config.bind.is_some() && !config.endpoints.is_empty() {
            anyhow::bail!("cannot specify both bind and endpoints");
        }

        if config.admission.development_only() && !config.development {
            anyhow::bail!("development-only admission requires explicit development mode");
        }
        if config.admission.allow_all()
            && config.listener_security != ListenerSecurityPolicy::Development
        {
            anyhow::bail!("allow-all admission is restricted to development listeners");
        }
        if !config.development {
            anyhow::ensure!(
                config.qlog_dir.is_none() && config.mlog_dir.is_none(),
                "per-session qlog/mlog files are development-only without bounded retention"
            );
            anyhow::ensure!(
                config
                    .endpoints
                    .iter()
                    .all(|endpoint| !endpoint.writes_per_connection_diagnostics()),
                "production relay endpoints cannot create unbounded per-session qlog files"
            );
            anyhow::ensure!(
                config.endpoints.iter().all(Endpoint::uses_stateless_retry),
                "production relay endpoints require QUIC stateless retry"
            );
            anyhow::ensure!(
                config
                    .endpoints
                    .iter()
                    .all(|endpoint| !endpoint.tls_key_logging_enabled()),
                "production relay endpoints cannot enable TLS key logging"
            );
        }
        anyhow::ensure!(
            !config.setup_timeout.is_zero(),
            "SETUP timeout must be positive"
        );
        anyhow::ensure!(
            !config.admission_timeout.is_zero(),
            "admission timeout must be positive"
        );
        anyhow::ensure!(
            !config.cleanup_timeout.is_zero(),
            "pre-admission cleanup timeout must be positive"
        );
        anyhow::ensure!(
            !config.session_close_timeout.is_zero(),
            "admission session-close timeout must be positive"
        );
        anyhow::ensure!(
            config.max_pending_admissions > 0,
            "pending admission limit must be positive"
        );
        anyhow::ensure!(
            config.max_pending_admissions <= tokio::sync::Semaphore::MAX_PERMITS,
            "pending admission limit exceeds the semaphore maximum"
        );
        anyhow::ensure!(
            config.max_active_sessions > 0,
            "active session limit must be positive"
        );
        anyhow::ensure!(
            config.max_active_sessions <= tokio::sync::Semaphore::MAX_PERMITS,
            "active session limit exceeds the semaphore maximum"
        );
        anyhow::ensure!(
            !config.token_revalidation_interval.is_zero(),
            "token revalidation interval must be positive"
        );
        let capacity = RelayCapacity::new(config.capacity_limits)?;
        config.tracks_limits.validate()?;
        let request_capacity = moq_transport::session::RequestCapacity::new(config.request_limits)?;

        if !config.development {
            anyhow::ensure!(
                config.listener_security != ListenerSecurityPolicy::Development,
                "development listener security cannot be used in production"
            );
            if matches!(
                config.listener_security,
                ListenerSecurityPolicy::TokenSubscriber
                    | ListenerSecurityPolicy::RawQuicTokenSubscriber
            ) {
                anyhow::ensure!(
                    config.admission.supports_production_token_leases(),
                    "production token listeners require an external replay- and lease-aware admission policy"
                );
                anyhow::ensure!(
                    config.admission.supports_atomic_token_admission(),
                    "production token listeners require atomic admission and lease acquisition"
                );
                anyhow::ensure!(
                    config.admission.supports_awaited_session_close(),
                    "production token listeners require awaited replay tombstoning and lease release"
                );
            }
            anyhow::ensure!(
                config.admission.supports_bounded_session_leases(),
                "production admission policies must provide bounded principal/tenant session leases"
            );
            anyhow::ensure!(
                config.tls.verifies_server_certificates(),
                "production relay mode forbids --tls-disable-verify for outbound connections"
            );
            anyhow::ensure!(
                config
                    .endpoints
                    .iter()
                    .all(Endpoint::verifies_server_certificates),
                "all production relay endpoints must verify outbound server certificates"
            );
        }

        let required_client_auth = match config.listener_security {
            ListenerSecurityPolicy::MutualTlsPublisher
            | ListenerSecurityPolicy::MutualTlsRelaySubscriber => {
                moq_native_ietf::tls::ClientAuthMode::Required
            }
            ListenerSecurityPolicy::TokenSubscriber
            | ListenerSecurityPolicy::RawQuicTokenSubscriber
            | ListenerSecurityPolicy::Development => moq_native_ietf::tls::ClientAuthMode::Disabled,
        };
        anyhow::ensure!(
            config.tls.client_auth_mode() == required_client_auth,
            "listener TLS client-auth mode does not match its security policy"
        );
        anyhow::ensure!(
            config
                .endpoints
                .iter()
                .all(|endpoint| endpoint.client_auth_mode() == required_client_auth),
            "endpoint TLS client-auth mode does not match listener security policy"
        );

        let endpoints = if let Some(bind) = config.bind {
            let endpoint = quic::Endpoint::new(quic::Config::new(
                bind,
                config.qlog_dir.clone(),
                config.tls.clone(),
            )?)?;
            vec![endpoint]
        } else {
            config.endpoints
        };

        if endpoints.is_empty() {
            anyhow::bail!("no endpoints available to start the server");
        }

        // Validate mlog directory if provided
        if let Some(mlog_dir) = &config.mlog_dir {
            if !mlog_dir.exists() {
                anyhow::bail!("mlog directory does not exist: {}", mlog_dir.display());
            }
            if !mlog_dir.is_dir() {
                anyhow::bail!("mlog path is not a directory: {}", mlog_dir.display());
            }
            tracing::info!("mlog output enabled: {}", mlog_dir.display());
        }

        // FIXME(itzmanish): have a generic filter to find endpoints for forward, remote etc.
        let remote_clients = endpoints
            .iter()
            .map(|endpoint| endpoint.client.clone())
            .collect::<Vec<_>>();

        // Create remote manager - uses coordinator for namespace lookups
        let remotes = RemoteManager::with_limits_and_capacity(
            config.coordinator.clone(),
            remote_clients,
            config.remote_limits,
            request_capacity.clone(),
        )?;

        Ok(Self {
            quic_endpoints: endpoints,
            announce_url: config.announce,
            mlog_dir: config.mlog_dir,
            locals,
            remotes,
            coordinator: config.coordinator,
            admission: config.admission,
            listener_security: config.listener_security,
            setup_timeout: config.setup_timeout,
            admission_timeout: config.admission_timeout,
            cleanup_timeout: config.cleanup_timeout,
            session_close_timeout: config.session_close_timeout,
            max_pending_admissions: config.max_pending_admissions,
            max_active_sessions: config.max_active_sessions,
            production: !config.development,
            token_revalidation_interval: config.token_revalidation_interval,
            capacity,
            tracks_limits: config.tracks_limits,
            request_capacity,
        })
    }

    /// Return aggregate capacity diagnostics without principal or scope labels.
    pub fn capacity_snapshot(&self) -> crate::RelayCapacitySnapshot {
        self.capacity.snapshot()
    }

    pub async fn remote_snapshot(&self) -> crate::RemoteManagerSnapshot {
        self.remotes.snapshot().await
    }

    /// Return a diagnostics handle that remains usable after moving this relay into `run`.
    pub fn diagnostics(&self) -> RelayDiagnostics {
        RelayDiagnostics {
            capacity: self.capacity.clone(),
            remotes: self.remotes.clone(),
            request_capacity: self.request_capacity.clone(),
        }
    }

    /// Run the relay server.
    pub async fn run(self) -> anyhow::Result<()> {
        self.run_until(CancellationToken::new()).await
    }

    /// Run until `shutdown` is cancelled, then stop accepting new sessions and
    /// await the bounded finalizer for every admitted session.
    pub async fn run_until(self, shutdown: CancellationToken) -> anyhow::Result<()> {
        let admission_reaper = AdmissionReaper::new();
        let Self {
            quic_endpoints,
            announce_url,
            mlog_dir,
            locals,
            remotes,
            coordinator,
            admission,
            listener_security,
            setup_timeout,
            admission_timeout,
            cleanup_timeout,
            session_close_timeout,
            max_pending_admissions,
            max_active_sessions,
            production,
            token_revalidation_interval,
            capacity,
            tracks_limits,
            request_capacity,
        } = self;

        let run_result: anyhow::Result<()> = async {
            let mut tasks = FuturesUnordered::new();
            tasks.push(
                report_retention_metrics(request_capacity.clone(), shutdown.clone()).boxed(),
            );

            // Use the remote manager for routing to remote relays.
            let remote_manager = remotes.clone();

            // Start the forwarder, if any
            let forward_producer = if let Some(url) = &announce_url {
                tracing::info!(
                    remote_url = %crate::redact_url_for_logging(url),
                    "forwarding PUBLISH_NAMESPACE messages"
                );

                // Establish a QUIC connection to the forward URL
                let (target, policy) = quic::compatibility_target(url)?;
                let connection = quic_endpoints[0]
                    .client
                    .connect_target(&target, policy, None)
                    .await
                    .context("failed to establish forward connection")?;

                // Create the MoQ session over the connection
                let (session, publisher, subscriber) =
                    moq_transport::session::Session::connect_with_capacity(
                        connection.session,
                        None,
                        connection.negotiated,
                        &request_capacity,
                    )
                    .await
                    .context("failed to establish forward session")?;

                // Use the connection path already validated and stored by Session::connect().
                // The forward session is scoped to whatever path the announce URL specifies.
                //
                // Note: the forward connection intentionally does not call
                // coordinator.resolve_scope(). The announce URL is operator-configured
                // (via --announce), not client-supplied, so it doesn't need the same
                // auth/permission checks that incoming client connections get. The
                // forward session always gets both Producer and Consumer (full
                // read-write) since it's acting as a relay peer, not a client.
                //
                // Limitation: all incoming scopes are forwarded to this single upstream scope.
                // Multi-scope forwarding (routing different incoming scopes to different
                // upstream paths) would require per-scope forward connections.
                let forward_scope = session.connection_path().map(|s| s.to_string());
                let forward_identity = RelayIdentity::operator(forward_scope.clone());

                let forward_coordinator = coordinator.clone();
                let session = Session {
                    session,
                    producer: Some(Producer::new_admitted(
                        publisher,
                        locals.clone(),
                        remote_manager.clone(),
                        forward_identity.clone(),
                        capacity.clone(),
                    )),
                    consumer: Some(Consumer::new_admitted(
                        subscriber,
                        locals.clone(),
                        forward_coordinator,
                        None,
                        forward_identity,
                        capacity.clone(),
                        tracks_limits,
                    )),
                    // Forward connections are always full read-write relay peers,
                    // so no reject loops needed.
                    reject_publishes: None,
                    reject_subscribes: None,
                };

                let forward_producer = session.producer.clone();

                let forward_shutdown = shutdown.clone();
                tasks.push(
                    async move {
                        tokio::select! {
                            result = session.run() => result.context("forwarding failed"),
                            _ = forward_shutdown.cancelled() => Ok(()),
                        }
                    }
                    .boxed(),
                );

                forward_producer
            } else {
                None
            };

            let servers: Vec<quic::Server> = quic_endpoints
                .into_iter()
                .map(|endpoint| endpoint.server.context("missing TLS certificate for server"))
                .collect::<anyhow::Result<_>>()?;

            // This will hold the futures for all our listening servers.
            let mut accepts: FuturesUnordered<ServerFuture> = FuturesUnordered::new();
            for mut server in servers {
                tracing::info!("listening on {}", server.local_addr()?);
                let pending_admissions = Arc::new(tokio::sync::Semaphore::new(
                    max_pending_admissions,
                ));
                let active_sessions =
                    Arc::new(tokio::sync::Semaphore::new(max_active_sessions));

                // Create a future, box it, and push it to the collection.
                accepts.push(
                    async move {
                        let conn = server.accept_connection().await.context("accept failed");
                        (conn, server, pending_admissions, active_sessions)
                    }
                    .boxed(),
                );
            }

            let mut terminal_error = None;
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    // This branch polls all the `accept` futures concurrently.
                    Some((conn_result, mut server, pending_admissions, active_sessions)) = accepts.next() => {
                        // An accept operation has completed.
                        // First, immediately queue up the next accept() call for this server.
                        let next_pending_admissions = pending_admissions.clone();
                        let next_active_sessions = active_sessions.clone();
                        accepts.push(
                            async move {
                                let conn = server.accept_connection().await.context("accept failed");
                                (conn, server, next_pending_admissions, next_active_sessions)
                            }
                            .boxed(),
                        );

                        let connection = match conn_result {
                            Ok(connection) => connection,
                            Err(error) => {
                                terminal_error = Some(error.context("failed to accept QUIC connection"));
                                break;
                            }
                        };
                        metrics::counter!("moq_relay_connections_total").increment(1);
                        let admission_permit = match pending_admissions.try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                let session = connection.session;
                                // Overload rejection is intentionally inline:
                                // `close` is nonblocking and we retain no
                                // per-connection cleanup future, so a flood
                                // cannot grow the relay task set.
                                session.close(
                                    moq_transport::session::SessionTerminationCode::InternalError.as_u32(),
                                    "pending admission capacity exhausted",
                                );
                                metrics::counter!("moq_relay_connection_errors_total", "stage" => "admission_capacity").increment(1);
                                metrics::counter!("moq_relay_connections_closed_total").increment(1);
                                continue;
                            }
                        };

                        let connection_id = connection.connection_id;
                        let negotiated = connection.negotiated;
                        let peer_identity = connection.peer_identity;
                        let conn = connection.session;

                        // Construct mlog path from connection ID if mlog directory is configured
                        let mlog_path = mlog_dir.as_ref()
                            .map(|dir| dir.join(format!("{}_server.mlog", connection_id)));

                        let locals = locals.clone();
                        let remotes = remote_manager.clone();
                        let forward = forward_producer.clone();
                        let coordinator = coordinator.clone();
                        let admission = admission.clone();
                        let active_sessions = active_sessions.clone();
                        let capacity = capacity.clone();
                        let request_capacity = request_capacity.clone();
                        let session_shutdown = shutdown.clone();
                        let session_reaper = admission_reaper.clone();

                        // Spawn a new task to handle the connection
                        tasks.push(async move {
                            let admission_permit = admission_permit;
                            // Track active connections - decrements when task completes
                            let _conn_guard = GaugeGuard::new("moq_relay_active_connections");

                            // Clone the raw connection so we can close it with a proper
                            // error code if scope resolution fails after the MoQ handshake.
                            let raw_conn = conn.clone();

                            // Create the MoQ session over the connection (setup handshake etc)
                            let (session, publisher, subscriber) = match tokio::time::timeout(
                                setup_timeout,
                                moq_transport::session::Session::accept_with_capacity(
                                    conn,
                                    mlog_path,
                                    negotiated,
                                    &request_capacity,
                                ),
                            ).await {
                                Ok(Ok(session)) => session,
                                Ok(Err(err)) => {
                                    tracing::warn!(error = %err, "failed to accept MoQ session: {}", err);
                                    metrics::counter!("moq_relay_connection_errors_total", "stage" => "session_accept").increment(1);
                                    close_and_wait(&raw_conn, moq_transport::session::SessionTerminationCode::ProtocolViolation.as_u32(), "invalid SETUP", cleanup_timeout).await;
                                    metrics::counter!("moq_relay_connections_closed_total").increment(1);
                                    return Ok(());
                                }
                                Err(_) => {
                                    tracing::warn!("timed out waiting for peer SETUP");
                                    metrics::counter!("moq_relay_connection_errors_total", "stage" => "setup_timeout").increment(1);
                                    close_and_wait(&raw_conn, moq_transport::session::SessionTerminationCode::InternalError.as_u32(), "SETUP timeout", cleanup_timeout).await;
                                    metrics::counter!("moq_relay_connections_closed_total").increment(1);
                                    return Ok(());
                                }
                            };

                            let moq_session = session;

                            if !listener_session_is_allowed(
                                listener_security,
                                moq_session.transport(),
                                moq_session.negotiated_transport().protocol,
                                moq_session.peer_setup_authorization(),
                            ) {
                                metrics::counter!(
                                    "moq_relay_connection_errors_total",
                                    "stage" => "listener_requirements"
                                )
                                .increment(1);
                                close_and_wait(
                                    &raw_conn,
                                    moq_transport::session::SessionTerminationCode::Unauthorized
                                        .as_u32(),
                                    "listener transport or SETUP authorization is not allowed for this role",
                                    cleanup_timeout,
                                )
                                .await;
                                metrics::counter!("moq_relay_connections_closed_total")
                                    .increment(1);
                                return Ok(());
                            }

                            // Reserve process capacity before an external policy mutates replay or
                            // distributed quota state.
                            let active_session_permit =
                                match active_sessions.try_acquire_owned() {
                                    Ok(permit) => permit,
                                    Err(_) => {
                                        metrics::counter!(
                                            "moq_relay_connection_errors_total",
                                            "stage" => "session_capacity"
                                        )
                                        .increment(1);
                                        close_and_wait(
                                            &raw_conn,
                                            moq_transport::session::SessionTerminationCode::InternalError
                                                .as_u32(),
                                            "active session capacity exhausted",
                                            cleanup_timeout,
                                        )
                                        .await;
                                        metrics::counter!("moq_relay_connections_closed_total")
                                            .increment(1);
                                        return Ok(());
                                    }
                                };

                            let session_id = match AdmissionSessionId::generate() {
                                Ok(session_id) => session_id,
                                Err(error) => {
                                    tracing::error!(%error, "failed to generate admission session ID");
                                    metrics::counter!(
                                        "moq_relay_connection_errors_total",
                                        "stage" => "session_id"
                                    )
                                    .increment(1);
                                    close_and_wait(
                                        &raw_conn,
                                        moq_transport::session::SessionTerminationCode::InternalError
                                            .as_u32(),
                                        "session admission unavailable",
                                        cleanup_timeout,
                                    )
                                    .await;
                                    metrics::counter!("moq_relay_connections_closed_total")
                                        .increment(1);
                                    return Ok(());
                                }
                            };
                            let connection_close_metric =
                                Arc::new(std::sync::atomic::AtomicBool::new(false));
                            let mut admission_attempt = AdmissionAttempt {
                                admission,
                                request: OwnedAdmissionRequest {
                                    session_id,
                                    peer_identity: peer_identity.clone(),
                                    target: moq_session.target().clone(),
                                    substrate: moq_session.transport(),
                                    negotiated_protocol: moq_session.negotiated_transport().protocol,
                                    setup_authorization: moq_session
                                        .peer_setup_authorization()
                                        .cloned(),
                                },
                                resources: AdmissionLifecycleResources {
                                    raw_conn: raw_conn.clone(),
                                    connection_close_metric: connection_close_metric.clone(),
                                    pending_admission_permit: Some(admission_permit),
                                    active_session_permit,
                                    cleanup_timeout,
                                    session_close_timeout,
                                },
                                reaper: session_reaper.clone(),
                            }
                            .spawn();
                            let admitted = match tokio::time::timeout(
                                admission_timeout,
                                &mut admission_attempt,
                            )
                            .await
                            {
                                Ok(Ok(Ok(admitted))) => admitted,
                                Ok(Ok(Err(error))) => {
                                    if should_log_admission_warning() {
                                        tracing::warn!(
                                            peer = ?peer_identity,
                                            target = %moq_session.target().redacted_for_logging(),
                                            substrate = ?moq_session.transport(),
                                            protocol = moq_session.negotiated_transport().protocol,
                                            %error,
                                            "session admission denied (warnings sampled)"
                                        );
                                    }
                                    metrics::counter!(
                                        "moq_relay_connection_errors_total",
                                        "stage" => "admission"
                                    )
                                    .increment(1);
                                    close_and_wait(
                                        &raw_conn,
                                        moq_transport::session::SessionTerminationCode::Unauthorized
                                            .as_u32(),
                                        "session admission denied",
                                        cleanup_timeout,
                                    )
                                    .await;
                                    record_connection_closed(&connection_close_metric);
                                    return Ok(());
                                }
                                Ok(Err(error)) => {
                                    tracing::error!(%error, "atomic admission task failed");
                                    metrics::counter!(
                                        "moq_relay_connection_errors_total",
                                        "stage" => "admission_task"
                                    )
                                    .increment(1);
                                    close_and_wait(
                                        &raw_conn,
                                        moq_transport::session::SessionTerminationCode::InternalError
                                            .as_u32(),
                                        "session admission failed",
                                        cleanup_timeout,
                                    )
                                    .await;
                                    record_connection_closed(&connection_close_metric);
                                    return Ok(());
                                }
                                Err(_) => {
                                    if should_log_admission_warning() {
                                        tracing::warn!(
                                            "atomic session admission timed out (warnings sampled)"
                                        );
                                    }
                                    metrics::counter!(
                                        "moq_relay_connection_errors_total",
                                        "stage" => "admission_timeout"
                                    )
                                    .increment(1);
                                    session_reaper.reap_late_admission(admission_attempt);
                                    close_and_wait(
                                        &raw_conn,
                                        moq_transport::session::SessionTerminationCode::Unauthorized
                                            .as_u32(),
                                        "session admission timeout",
                                        cleanup_timeout,
                                    )
                                    .await;
                                    record_connection_closed(&connection_close_metric);
                                    return Ok(());
                                }
                            };

                            run_admitted_connection(
                                admitted,
                                moq_session,
                                publisher,
                                subscriber,
                                AdmittedConnectionContext {
                                    raw_conn,
                                    peer_identity,
                                    listener_security,
                                    production,
                                    coordinator,
                                    locals,
                                    remotes,
                                    forward,
                                    capacity,
                                    tracks_limits,
                                    admission_timeout,
                                    cleanup_timeout,
                                    token_revalidation_interval,
                                    shutdown: session_shutdown,
                                },
                            )
                            .await;

                            Ok(())
                        }.boxed());
                    },
                    result = tasks.next(), if !tasks.is_empty() => {
                        if let Some(Err(error)) = result {
                            terminal_error = Some(error);
                            break;
                        }
                    },
                }
            }

            shutdown.cancel();
            drop(accepts);
            while let Some(result) = tasks.next().await {
                if let Err(error) = result {
                    if terminal_error.is_none() {
                        terminal_error = Some(error);
                    } else {
                        tracing::warn!(%error, "relay task failed during drain");
                    }
                }
            }
            if let Some(error) = terminal_error {
                Err(error)
            } else {
                Ok(())
            }
        }
        .await;

        admission_reaper.drain().await;
        remotes.shutdown().await;
        if let Err(error) = coordinator.shutdown().await {
            if run_result.is_ok() {
                return Err(anyhow::Error::new(error).context("coordinator shutdown failed"));
            }
            tracing::warn!(%error, "coordinator shutdown failed after relay error");
        }
        run_result
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use async_trait::async_trait;
    use moq_native_ietf::tls;
    use moq_transport::{coding::TrackNamespace, session::SetupAuthorization};
    use rcgen::{
        BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair,
        KeyUsagePurpose,
    };
    use time::OffsetDateTime;

    use crate::{
        AdmissionClaims, AdmissionCloseError, AdmissionError, AdmissionLease, AdmissionPrincipal,
        AuthenticationMethod, CoordinatorError, CoordinatorResult, NamespaceOrigin,
        NamespaceRegistration,
    };

    #[derive(Default)]
    struct CountingCoordinator {
        resolve_calls: AtomicUsize,
        mutation_calls: AtomicUsize,
        lookup_calls: AtomicUsize,
        resolved_scope: Option<&'static str>,
        resolve_started: Option<Arc<tokio::sync::Semaphore>>,
        resolve_release: Option<Arc<tokio::sync::Semaphore>>,
        panic_on_resolve: bool,
    }

    #[async_trait]
    impl Coordinator for CountingCoordinator {
        async fn resolve_scope(
            &self,
            _connection_path: Option<&str>,
        ) -> CoordinatorResult<Option<crate::ScopeInfo>> {
            self.resolve_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(started) = &self.resolve_started {
                started.add_permits(1);
            }
            if let Some(release) = &self.resolve_release {
                let permit = release
                    .acquire()
                    .await
                    .map_err(|error| CoordinatorError::Other(error.into()))?;
                permit.forget();
            }
            if self.panic_on_resolve {
                panic!("forced coordinator panic after admission grant");
            }
            Ok(self.resolved_scope.map(|scope_id| crate::ScopeInfo {
                scope_id: scope_id.to_string(),
                permissions: crate::ScopePermissions::ReadWrite,
            }))
        }

        async fn register_namespace(
            &self,
            _scope: Option<&str>,
            _namespace: &TrackNamespace,
        ) -> CoordinatorResult<NamespaceRegistration> {
            self.mutation_calls.fetch_add(1, Ordering::SeqCst);
            Ok(NamespaceRegistration::new(()))
        }

        async fn unregister_namespace(
            &self,
            _scope: Option<&str>,
            _namespace: &TrackNamespace,
        ) -> CoordinatorResult<()> {
            self.mutation_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn lookup(
            &self,
            _scope: Option<&str>,
            _namespace: &TrackNamespace,
        ) -> CoordinatorResult<(NamespaceOrigin, Option<quic::Client>)> {
            self.lookup_calls.fetch_add(1, Ordering::SeqCst);
            Err(CoordinatorError::NamespaceNotFound)
        }
    }

    struct RecordingAdmission {
        calls: AtomicUsize,
        completions: AtomicUsize,
        context_valid: AtomicBool,
        delay: Duration,
        allow: bool,
    }

    impl RecordingAdmission {
        fn deny() -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                completions: AtomicUsize::new(0),
                context_valid: AtomicBool::new(false),
                delay: Duration::ZERO,
                allow: false,
            })
        }

        fn slow(delay: Duration) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                completions: AtomicUsize::new(0),
                context_valid: AtomicBool::new(false),
                delay,
                allow: false,
            })
        }
    }

    #[async_trait]
    impl SessionAdmission for RecordingAdmission {
        async fn admit(
            &self,
            request: AdmissionRequest<'_>,
        ) -> Result<AdmissionDecision, AdmissionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.context_valid.store(
                request.target.routing_path() == Some("/secure")
                    && request.substrate == moq_transport::session::Transport::RawQuic
                    && request.negotiated_protocol == "moqt-19"
                    && request
                        .setup_authorization
                        .is_some_and(|token| token.as_bytes() == b"test-token")
                    && matches!(request.peer_identity, tls::PeerIdentity::Anonymous),
                Ordering::SeqCst,
            );
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            if !self.allow {
                self.completions.fetch_add(1, Ordering::SeqCst);
                return Err(AdmissionError::PolicyDenied);
            }
            let decision = AdmissionDecision::new(
                AdmissionPrincipal::new("development-test", AuthenticationMethod::Development)
                    .map_err(|_| AdmissionError::PolicyDenied)?,
                AdmissionClaims {
                    scope: None,
                    publish: true,
                    subscribe: true,
                    expires_at_unix_seconds: None,
                    token_id: None,
                },
            )
            .map_err(|_| AdmissionError::PolicyDenied);
            self.completions.fetch_add(1, Ordering::SeqCst);
            decision
        }
    }

    #[derive(Default)]
    struct LeaseAdmission {
        revalidation_calls: Arc<AtomicUsize>,
    }

    impl LeaseAdmission {
        fn admitted(&self, decision: AdmissionDecision) -> AdmittedSession {
            AdmittedSession::new(
                decision,
                Box::new(RevalidationLease {
                    calls: self.revalidation_calls.clone(),
                }),
            )
        }
    }

    struct RevalidationLease {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl AdmissionLease for RevalidationLease {
        async fn revalidate(&self, _now_unix_seconds: u64) -> Result<(), AdmissionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait]
    impl SessionAdmission for LeaseAdmission {
        async fn admit(
            &self,
            _request: AdmissionRequest<'_>,
        ) -> Result<AdmissionDecision, AdmissionError> {
            Err(AdmissionError::PolicyDenied)
        }

        fn supports_production_token_leases(&self) -> bool {
            true
        }

        fn supports_bounded_session_leases(&self) -> bool {
            true
        }

        async fn revalidate(&self, _decision: &AdmissionDecision) -> Result<(), AdmissionError> {
            self.revalidation_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct AtomicOnlyAdmission;

    #[async_trait]
    impl SessionAdmission for AtomicOnlyAdmission {
        async fn admit(
            &self,
            _request: AdmissionRequest<'_>,
        ) -> Result<AdmissionDecision, AdmissionError> {
            Err(AdmissionError::PolicyDenied)
        }

        fn supports_production_token_leases(&self) -> bool {
            true
        }

        fn supports_bounded_session_leases(&self) -> bool {
            true
        }

        fn supports_atomic_token_admission(&self) -> bool {
            true
        }
    }

    struct ScopedDevelopmentAdmission;

    #[async_trait]
    impl SessionAdmission for ScopedDevelopmentAdmission {
        async fn admit(
            &self,
            _request: AdmissionRequest<'_>,
        ) -> Result<AdmissionDecision, AdmissionError> {
            AdmissionDecision::new(
                AdmissionPrincipal::new("scoped-test", AuthenticationMethod::Development)
                    .map_err(|_| AdmissionError::PolicyDenied)?,
                AdmissionClaims {
                    scope: Some("/unknown-scope".into()),
                    publish: true,
                    subscribe: true,
                    expires_at_unix_seconds: None,
                    token_id: None,
                },
            )
            .map_err(|_| AdmissionError::PolicyDenied)
        }
    }

    struct CapacityAdmission {
        calls: AtomicUsize,
        leases: Arc<AtomicUsize>,
    }

    impl CapacityAdmission {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                leases: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    struct CapacityLease(Arc<AtomicUsize>);

    impl AdmissionLease for CapacityLease {}

    impl Drop for CapacityLease {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl SessionAdmission for CapacityAdmission {
        async fn admit(
            &self,
            _request: AdmissionRequest<'_>,
        ) -> Result<AdmissionDecision, AdmissionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            AdmissionDecision::new(
                AdmissionPrincipal::new("capacity-test", AuthenticationMethod::Development)
                    .map_err(|_| AdmissionError::PolicyDenied)?,
                AdmissionClaims {
                    scope: None,
                    publish: true,
                    subscribe: true,
                    expires_at_unix_seconds: None,
                    token_id: None,
                },
            )
            .map_err(|_| AdmissionError::PolicyDenied)
        }

        async fn acquire_session_lease(
            &self,
            _decision: &AdmissionDecision,
        ) -> Result<Box<dyn AdmissionLease>, AdmissionError> {
            self.leases.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(CapacityLease(self.leases.clone())))
        }
    }

    #[derive(Clone, Copy)]
    enum LifecycleDecision {
        Valid,
        ScopedUnknown,
        InvalidRole,
        ProductionToken,
        ProductionPublishingToken,
    }

    struct LifecycleState {
        admissions: AtomicUsize,
        active_leases: AtomicUsize,
        close_calls: AtomicUsize,
        dropped_leases: AtomicUsize,
        close_blocked: AtomicBool,
        close_error: AtomicBool,
        revalidate_ok: AtomicBool,
        grant_blocked: AtomicBool,
        close_release: tokio::sync::Semaphore,
        grant_release: tokio::sync::Semaphore,
        session_ids: Mutex<Vec<String>>,
        close_reasons: Mutex<Vec<AdmissionCloseReason>>,
    }

    impl LifecycleState {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                admissions: AtomicUsize::new(0),
                active_leases: AtomicUsize::new(0),
                close_calls: AtomicUsize::new(0),
                dropped_leases: AtomicUsize::new(0),
                close_blocked: AtomicBool::new(false),
                close_error: AtomicBool::new(false),
                revalidate_ok: AtomicBool::new(true),
                grant_blocked: AtomicBool::new(false),
                close_release: tokio::sync::Semaphore::new(0),
                grant_release: tokio::sync::Semaphore::new(0),
                session_ids: Mutex::new(Vec::new()),
                close_reasons: Mutex::new(Vec::new()),
            })
        }
    }

    struct LifecycleLease {
        state: Arc<LifecycleState>,
        closed: bool,
    }

    #[async_trait]
    impl AdmissionLease for LifecycleLease {
        async fn revalidate(&self, _now_unix_seconds: u64) -> Result<(), AdmissionError> {
            if self.state.revalidate_ok.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(AdmissionError::PolicyDenied)
            }
        }

        async fn close(
            &mut self,
            context: AdmissionCloseContext,
        ) -> Result<(), AdmissionCloseError> {
            if self.closed {
                return Ok(());
            }
            self.state.close_calls.fetch_add(1, Ordering::SeqCst);
            self.state
                .close_reasons
                .lock()
                .unwrap()
                .push(context.reason);
            if self.state.close_blocked.load(Ordering::SeqCst) {
                let permit = self
                    .state
                    .close_release
                    .acquire()
                    .await
                    .map_err(|_| AdmissionCloseError::LeaseReleaseUnavailable)?;
                permit.forget();
            }
            if self.state.close_error.load(Ordering::SeqCst) {
                return Err(AdmissionCloseError::ReplayFinalizeUnavailable);
            }
            self.closed = true;
            Ok(())
        }
    }

    impl Drop for LifecycleLease {
        fn drop(&mut self) {
            self.state.active_leases.fetch_sub(1, Ordering::SeqCst);
            self.state.dropped_leases.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct LifecycleAdmission {
        state: Arc<LifecycleState>,
        decision: LifecycleDecision,
    }

    impl LifecycleAdmission {
        fn new(decision: LifecycleDecision) -> Arc<Self> {
            Arc::new(Self {
                state: LifecycleState::new(),
                decision,
            })
        }

        fn decision(&self) -> Result<AdmissionDecision, AdmissionError> {
            let (method, scope) = match self.decision {
                LifecycleDecision::Valid => (AuthenticationMethod::Development, None),
                LifecycleDecision::ScopedUnknown => (
                    AuthenticationMethod::Development,
                    Some("/unknown-scope".into()),
                ),
                LifecycleDecision::InvalidRole => (AuthenticationMethod::SetupToken, None),
                LifecycleDecision::ProductionToken => {
                    (AuthenticationMethod::SetupToken, Some("/secure".into()))
                }
                LifecycleDecision::ProductionPublishingToken => {
                    (AuthenticationMethod::SetupToken, Some("/secure".into()))
                }
            };
            AdmissionDecision::new(
                AdmissionPrincipal::new("lifecycle-test", method)
                    .map_err(|_| AdmissionError::PolicyDenied)?,
                if matches!(
                    self.decision,
                    LifecycleDecision::ProductionToken
                        | LifecycleDecision::ProductionPublishingToken
                ) {
                    AdmissionClaims {
                        scope,
                        publish: matches!(
                            self.decision,
                            LifecycleDecision::ProductionPublishingToken
                        ),
                        subscribe: matches!(self.decision, LifecycleDecision::ProductionToken),
                        expires_at_unix_seconds: Some(unix_now().saturating_add(60)),
                        token_id: Some("lifecycle-jti".into()),
                    }
                } else {
                    AdmissionClaims {
                        scope,
                        publish: true,
                        subscribe: true,
                        expires_at_unix_seconds: None,
                        token_id: None,
                    }
                },
            )
            .map_err(|_| AdmissionError::PolicyDenied)
        }
    }

    #[async_trait]
    impl SessionAdmission for LifecycleAdmission {
        async fn admit(
            &self,
            _request: AdmissionRequest<'_>,
        ) -> Result<AdmissionDecision, AdmissionError> {
            self.decision()
        }

        async fn admit_session(
            &self,
            request: AdmissionRequest<'_>,
        ) -> Result<AdmittedSession, AdmissionError> {
            let decision = self.decision()?;
            self.state.admissions.fetch_add(1, Ordering::SeqCst);
            self.state.active_leases.fetch_add(1, Ordering::SeqCst);
            self.state
                .session_ids
                .lock()
                .unwrap()
                .push(request.session_id.as_str().to_string());
            if self.state.grant_blocked.load(Ordering::SeqCst) {
                let permit = self
                    .state
                    .grant_release
                    .acquire()
                    .await
                    .map_err(|_| AdmissionError::PolicyDenied)?;
                permit.forget();
            }
            Ok(AdmittedSession::new(
                decision,
                Box::new(LifecycleLease {
                    state: self.state.clone(),
                    closed: false,
                }),
            ))
        }

        fn supports_bounded_session_leases(&self) -> bool {
            true
        }

        fn supports_production_token_leases(&self) -> bool {
            true
        }

        fn supports_atomic_token_admission(&self) -> bool {
            true
        }

        fn supports_awaited_session_close(&self) -> bool {
            true
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FakeReplayState {
        Claimed,
        Tombstoned,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FakeReplayCloseMode {
        Success = 0,
        Error = 1,
        Block = 2,
    }

    struct FakeReplayStore {
        entries: Mutex<HashMap<Vec<u8>, (String, FakeReplayState)>>,
        close_mode: AtomicUsize,
        close_calls: AtomicUsize,
        close_release: tokio::sync::Semaphore,
    }

    impl FakeReplayStore {
        fn new(mode: FakeReplayCloseMode) -> Arc<Self> {
            Arc::new(Self {
                entries: Mutex::new(HashMap::new()),
                close_mode: AtomicUsize::new(mode as usize),
                close_calls: AtomicUsize::new(0),
                close_release: tokio::sync::Semaphore::new(0),
            })
        }

        fn mode(&self) -> FakeReplayCloseMode {
            match self.close_mode.load(Ordering::SeqCst) {
                1 => FakeReplayCloseMode::Error,
                2 => FakeReplayCloseMode::Block,
                _ => FakeReplayCloseMode::Success,
            }
        }

        fn set_mode(&self, mode: FakeReplayCloseMode) {
            self.close_mode.store(mode as usize, Ordering::SeqCst);
        }
    }

    struct FakeReplayLease {
        store: Arc<FakeReplayStore>,
        credential: Vec<u8>,
        session_id: String,
        closed: bool,
    }

    #[async_trait]
    impl AdmissionLease for FakeReplayLease {
        async fn revalidate(&self, _now_unix_seconds: u64) -> Result<(), AdmissionError> {
            let entries = self.store.entries.lock().unwrap();
            match entries.get(&self.credential) {
                Some((owner, FakeReplayState::Claimed)) if owner == &self.session_id => Ok(()),
                _ => Err(AdmissionError::PolicyDenied),
            }
        }

        async fn close(
            &mut self,
            _context: AdmissionCloseContext,
        ) -> Result<(), AdmissionCloseError> {
            if self.closed {
                return Ok(());
            }
            {
                let mut entries = self.store.entries.lock().unwrap();
                let Some((owner, state)) = entries.get_mut(&self.credential) else {
                    return Err(AdmissionCloseError::InvalidState);
                };
                if owner != &self.session_id {
                    return Err(AdmissionCloseError::OwnershipMismatch);
                }
                // Tombstone before any await or injected failure. Cancellation
                // can therefore never make this credential reusable.
                *state = FakeReplayState::Tombstoned;
            }
            self.store.close_calls.fetch_add(1, Ordering::SeqCst);
            match self.store.mode() {
                FakeReplayCloseMode::Success => {
                    self.closed = true;
                    Ok(())
                }
                FakeReplayCloseMode::Error => Err(AdmissionCloseError::ReplayFinalizeUnavailable),
                FakeReplayCloseMode::Block => {
                    let permit = self
                        .store
                        .close_release
                        .acquire()
                        .await
                        .map_err(|_| AdmissionCloseError::LeaseReleaseUnavailable)?;
                    permit.forget();
                    self.closed = true;
                    Ok(())
                }
            }
        }
    }

    struct FakeReplayAdmission {
        store: Arc<FakeReplayStore>,
    }

    #[async_trait]
    impl SessionAdmission for FakeReplayAdmission {
        async fn admit(
            &self,
            _request: AdmissionRequest<'_>,
        ) -> Result<AdmissionDecision, AdmissionError> {
            Err(AdmissionError::PolicyDenied)
        }

        async fn admit_session(
            &self,
            request: AdmissionRequest<'_>,
        ) -> Result<AdmittedSession, AdmissionError> {
            let credential = request
                .setup_authorization
                .ok_or(AdmissionError::PolicyDenied)?
                .as_bytes()
                .to_vec();
            let session_id = request.session_id.as_str().to_string();
            {
                let mut entries = self.store.entries.lock().unwrap();
                if entries.contains_key(&credential) {
                    return Err(AdmissionError::PolicyDenied);
                }
                entries.insert(
                    credential.clone(),
                    (session_id.clone(), FakeReplayState::Claimed),
                );
            }
            let token_id = hex::encode(&credential);
            let decision = AdmissionDecision::new(
                AdmissionPrincipal::new("fake-replay", AuthenticationMethod::SetupToken)
                    .map_err(|_| AdmissionError::PolicyDenied)?,
                AdmissionClaims {
                    scope: Some("/secure".into()),
                    publish: false,
                    subscribe: true,
                    expires_at_unix_seconds: Some(unix_now().saturating_add(60)),
                    token_id: Some(token_id),
                },
            )
            .map_err(|_| AdmissionError::PolicyDenied)?;
            Ok(AdmittedSession::new(
                decision,
                Box::new(FakeReplayLease {
                    store: self.store.clone(),
                    credential,
                    session_id,
                    closed: false,
                }),
            ))
        }

        fn supports_production_token_leases(&self) -> bool {
            true
        }

        fn supports_bounded_session_leases(&self) -> bool {
            true
        }

        fn supports_atomic_token_admission(&self) -> bool {
            true
        }

        fn supports_awaited_session_close(&self) -> bool {
            true
        }
    }

    async fn fake_replay_admit(
        admission: &FakeReplayAdmission,
        session_id: &AdmissionSessionId,
        credential: &[u8],
    ) -> Result<AdmittedSession, AdmissionError> {
        let peer = tls::PeerIdentity::Anonymous;
        let target: moq_transport::session::SessionTarget =
            "moqt://relay.example/secure".parse().unwrap();
        let authorization = SetupAuthorization::new(credential).unwrap();
        admission
            .admit_session(AdmissionRequest {
                session_id,
                peer_identity: &peer,
                target: &target,
                substrate: moq_transport::session::Transport::WebTransport,
                negotiated_protocol: "moqt-19",
                setup_authorization: Some(&authorization),
            })
            .await
    }

    async fn wait_for_counter(counter: &AtomicUsize, expected: usize) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(2), async {
            while counter.load(Ordering::SeqCst) != expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .with_context(|| format!("counter did not reach {expected}"))?;
        Ok(())
    }

    async fn lifecycle_admitted(admission: &LifecycleAdmission) -> AdmittedSession {
        let session_id = AdmissionSessionId::generate().unwrap();
        let peer = tls::PeerIdentity::Anonymous;
        let target: moq_transport::session::SessionTarget =
            "moqt://relay.example/lifecycle".parse().unwrap();
        let authorization = SetupAuthorization::new(b"test-token").unwrap();
        admission
            .admit_session(AdmissionRequest {
                session_id: &session_id,
                peer_identity: &peer,
                target: &target,
                substrate: moq_transport::session::Transport::WebTransport,
                negotiated_protocol: "moqt-19",
                setup_authorization: Some(&authorization),
            })
            .await
            .unwrap()
    }

    fn token_decision(
        expires_at_unix_seconds: Option<u64>,
        scope: Option<&str>,
        token_id: Option<&str>,
    ) -> AdmissionDecision {
        AdmissionDecision::new(
            AdmissionPrincipal::new("token-test", AuthenticationMethod::SetupToken).unwrap(),
            AdmissionClaims {
                scope: scope.map(str::to_owned),
                publish: false,
                subscribe: true,
                expires_at_unix_seconds,
                token_id: token_id.map(str::to_owned),
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn exact_token_expiry_boundary_skips_revalidation() {
        let admission = LeaseAdmission::default();
        let decision = token_decision(Some(101), Some("tenant/broadcast"), Some("jti-1"));
        let admitted = admission.admitted(decision);
        let clock = Arc::new(AtomicU64::new(100));
        let sleep_clock = clock.clone();

        let result = monitor_token_lease_with_clock(
            &admitted,
            Duration::from_secs(10),
            Duration::from_secs(30),
            || clock.load(Ordering::SeqCst),
            move |duration| {
                let sleep_clock = sleep_clock.clone();
                async move {
                    assert_eq!(duration, Duration::from_secs(1));
                    sleep_clock.store(101, Ordering::SeqCst);
                }
            },
        )
        .await;

        assert_eq!(result, Err(AdmissionError::PolicyDenied));
        assert_eq!(admission.revalidation_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn token_expiring_during_admission_is_rejected_before_activation() {
        let admission = LeaseAdmission::default();
        let decision = token_decision(Some(101), Some("tenant/broadcast"), Some("jti-1"));
        let admitted = admission.admitted(decision);
        let result =
            revalidate_token_before_activation(&admitted, Duration::from_secs(30), 101).await;
        assert_eq!(result, Err(AdmissionError::PolicyDenied));
        assert_eq!(admission.revalidation_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn production_token_claims_fail_closed() {
        let authorization = SetupAuthorization::new(b"test-token").unwrap();
        let identity = tls::PeerIdentity::Anonymous;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let valid = token_decision(Some(now + 60), Some("tenant/broadcast"), Some("jti-1"));
        let invalid = [
            token_decision(None, Some("tenant/broadcast"), Some("jti-1")),
            token_decision(Some(now + 60), None, Some("jti-1")),
            token_decision(Some(now + 60), Some("tenant/broadcast"), None),
            token_decision(Some(now), Some("tenant/broadcast"), Some("jti-1")),
        ];

        for (policy, substrate) in [
            (
                ListenerSecurityPolicy::TokenSubscriber,
                moq_transport::session::Transport::WebTransport,
            ),
            (
                ListenerSecurityPolicy::RawQuicTokenSubscriber,
                moq_transport::session::Transport::RawQuic,
            ),
        ] {
            assert!(listener_decision_is_valid(
                policy,
                &identity,
                Some(&authorization),
                substrate,
                std::str::from_utf8(moq_transport::setup::ALPN).unwrap(),
                &valid,
                true,
            ));
            for invalid in &invalid {
                assert!(!listener_decision_is_valid(
                    policy,
                    &identity,
                    Some(&authorization),
                    substrate,
                    std::str::from_utf8(moq_transport::setup::ALPN).unwrap(),
                    invalid,
                    true,
                ));
            }

            assert!(!listener_decision_is_valid(
                policy,
                &identity,
                None,
                substrate,
                std::str::from_utf8(moq_transport::setup::ALPN).unwrap(),
                &valid,
                true,
            ));
            assert!(!listener_decision_is_valid(
                policy,
                &identity,
                Some(&authorization),
                substrate,
                "moqt-18",
                &valid,
                true,
            ));
        }

        let malformed = AdmissionDecision {
            principal: AdmissionPrincipal::new("malformed", AuthenticationMethod::SetupToken)
                .unwrap(),
            claims: AdmissionClaims {
                scope: Some(String::new()),
                publish: false,
                subscribe: true,
                expires_at_unix_seconds: Some(now + 60),
                token_id: Some("jti-malformed".into()),
            },
        };
        assert!(!listener_decision_is_valid(
            ListenerSecurityPolicy::TokenSubscriber,
            &identity,
            Some(&authorization),
            moq_transport::session::Transport::WebTransport,
            std::str::from_utf8(moq_transport::setup::ALPN).unwrap(),
            &malformed,
            true,
        ));

        assert!(!listener_decision_is_valid(
            ListenerSecurityPolicy::TokenSubscriber,
            &identity,
            Some(&authorization),
            moq_transport::session::Transport::RawQuic,
            std::str::from_utf8(moq_transport::setup::ALPN).unwrap(),
            &valid,
            true,
        ));
        assert!(!listener_decision_is_valid(
            ListenerSecurityPolicy::RawQuicTokenSubscriber,
            &identity,
            Some(&authorization),
            moq_transport::session::Transport::WebTransport,
            std::str::from_utf8(moq_transport::setup::ALPN).unwrap(),
            &valid,
            true,
        ));

        let publishing = AdmissionDecision::new(
            AdmissionPrincipal::new("publishing-token", AuthenticationMethod::SetupToken).unwrap(),
            AdmissionClaims {
                scope: Some("tenant/broadcast".into()),
                publish: true,
                subscribe: false,
                expires_at_unix_seconds: Some(now + 60),
                token_id: Some("jti-publisher".into()),
            },
        )
        .unwrap();
        for (policy, substrate) in [
            (
                ListenerSecurityPolicy::TokenSubscriber,
                moq_transport::session::Transport::WebTransport,
            ),
            (
                ListenerSecurityPolicy::RawQuicTokenSubscriber,
                moq_transport::session::Transport::RawQuic,
            ),
        ] {
            assert!(!listener_decision_is_valid(
                policy,
                &identity,
                Some(&authorization),
                substrate,
                std::str::from_utf8(moq_transport::setup::ALPN).unwrap(),
                &publishing,
                true,
            ));
        }
    }

    #[test]
    fn mtls_certificate_roles_enforce_exact_least_privilege_claims() {
        let principal =
            || AdmissionPrincipal::new("custom-mtls", AuthenticationMethod::MutualTls).unwrap();
        let publisher = AdmissionDecision::new(
            principal(),
            AdmissionClaims {
                scope: Some("/tenant/live".into()),
                publish: true,
                subscribe: false,
                expires_at_unix_seconds: None,
                token_id: None,
            },
        )
        .unwrap();
        assert!(decision_matches_listener_role(
            ListenerSecurityPolicy::MutualTlsPublisher,
            &publisher,
            true,
            0,
        ));

        let bidirectional = AdmissionDecision::new(
            principal(),
            AdmissionClaims {
                subscribe: true,
                ..publisher.claims.clone()
            },
        )
        .unwrap();
        assert!(!decision_matches_listener_role(
            ListenerSecurityPolicy::MutualTlsPublisher,
            &bidirectional,
            true,
            0,
        ));

        let subscriber = AdmissionDecision::new(
            principal(),
            AdmissionClaims {
                scope: Some("/tenant/live".into()),
                publish: false,
                subscribe: true,
                expires_at_unix_seconds: None,
                token_id: None,
            },
        )
        .unwrap();
        assert!(decision_matches_listener_role(
            ListenerSecurityPolicy::MutualTlsRelaySubscriber,
            &subscriber,
            true,
            0,
        ));
        assert!(!decision_matches_listener_role(
            ListenerSecurityPolicy::MutualTlsRelaySubscriber,
            &publisher,
            true,
            0,
        ));
        assert!(!decision_matches_listener_role(
            ListenerSecurityPolicy::MutualTlsRelaySubscriber,
            &bidirectional,
            true,
            0,
        ));

        let token_metadata = AdmissionDecision::new(
            principal(),
            AdmissionClaims {
                token_id: Some("unexpected-token".into()),
                ..subscriber.claims.clone()
            },
        )
        .unwrap();
        assert!(!decision_matches_listener_role(
            ListenerSecurityPolicy::MutualTlsRelaySubscriber,
            &token_metadata,
            true,
            0,
        ));
    }

    #[test]
    fn mtls_relay_subscriber_requires_raw_quic_draft_19_without_setup_token() {
        let policy = ListenerSecurityPolicy::MutualTlsRelaySubscriber;
        let raw = moq_transport::session::Transport::RawQuic;
        let webtransport = moq_transport::session::Transport::WebTransport;
        let protocol = std::str::from_utf8(moq_transport::setup::ALPN).unwrap();
        assert!(listener_session_is_allowed(policy, raw, protocol, None));
        assert!(!listener_session_is_allowed(
            policy,
            webtransport,
            protocol,
            None
        ));
        assert!(!listener_session_is_allowed(policy, raw, "moqt-18", None));
        assert!(!listener_session_is_allowed(
            policy,
            raw,
            protocol,
            Some(&SetupAuthorization::new(b"ambiguous-auth").unwrap()),
        ));
    }

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("rvoip-moq-native")
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    struct ProductionPki {
        _directory: tempfile::TempDir,
        ca: PathBuf,
        server_cert: PathBuf,
        server_key: PathBuf,
        client_cert: PathBuf,
        client_key: PathBuf,
        client_fingerprint: String,
    }

    fn production_pki() -> anyhow::Result<ProductionPki> {
        let directory = tempfile::tempdir()?;
        let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
        ];
        ca_params.not_before = OffsetDateTime::now_utc() - time::Duration::days(1);
        ca_params.not_after = OffsetDateTime::now_utc() + time::Duration::days(30);
        let ca_key = KeyPair::generate()?;
        let ca = ca_params.self_signed(&ca_key)?;
        let ca_path = directory.path().join("ca.pem");
        fs::write(&ca_path, ca.pem())?;

        let identity = |name: &str,
                        dns_name: &str,
                        usage: ExtendedKeyUsagePurpose|
         -> anyhow::Result<(PathBuf, PathBuf, Vec<u8>)> {
            let mut params = CertificateParams::new(vec![dns_name.to_string()])?;
            params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
            params.extended_key_usages = vec![usage];
            params.not_before = OffsetDateTime::now_utc() - time::Duration::days(1);
            params.not_after = OffsetDateTime::now_utc() + time::Duration::days(1);
            let key = KeyPair::generate()?;
            let certificate = params.signed_by(&key, &ca, &ca_key)?;
            let cert_path = directory.path().join(format!("{name}-cert.pem"));
            let key_path = directory.path().join(format!("{name}-key.pem"));
            fs::write(&cert_path, certificate.pem())?;
            fs::write(&key_path, key.serialize_pem())?;
            Ok((cert_path, key_path, certificate.der().as_ref().to_vec()))
        };
        let (server_cert, server_key, _) =
            identity("server", "localhost", ExtendedKeyUsagePurpose::ServerAuth)?;
        let (client_cert, client_key, client_der) =
            identity("client", "client.test", ExtendedKeyUsagePurpose::ClientAuth)?;
        let fingerprint = ring::digest::digest(&ring::digest::SHA256, &client_der);

        Ok(ProductionPki {
            _directory: directory,
            ca: ca_path,
            server_cert,
            server_key,
            client_cert,
            client_key,
            client_fingerprint: hex::encode(fingerprint.as_ref()),
        })
    }

    fn development_tls() -> anyhow::Result<tls::Config> {
        tls::Args {
            cert: vec![fixture("localhost-cert.pem")],
            key: vec![fixture("localhost-key.pem")],
            disable_verify: true,
            ..Default::default()
        }
        .load()
    }

    struct RunningRelay {
        client: quic::Client,
        target: moq_transport::session::SessionTarget,
        task: tokio::task::JoinHandle<anyhow::Result<()>>,
        shutdown: CancellationToken,
        coordinator: Arc<CountingCoordinator>,
    }

    async fn start_development_relay(
        admission: Arc<dyn SessionAdmission>,
        setup_timeout: Duration,
        admission_timeout: Duration,
        max_pending_admissions: usize,
        max_active_sessions: usize,
    ) -> anyhow::Result<RunningRelay> {
        start_development_relay_with_coordinator(
            admission,
            setup_timeout,
            admission_timeout,
            max_pending_admissions,
            max_active_sessions,
            Arc::new(CountingCoordinator::default()),
        )
        .await
    }

    async fn start_development_relay_with_coordinator(
        admission: Arc<dyn SessionAdmission>,
        setup_timeout: Duration,
        admission_timeout: Duration,
        max_pending_admissions: usize,
        max_active_sessions: usize,
        coordinator: Arc<CountingCoordinator>,
    ) -> anyhow::Result<RunningRelay> {
        let tls = development_tls()?;
        let endpoint = Endpoint::new(quic::Config::new(
            "127.0.0.1:0".parse()?,
            None,
            tls.clone(),
        )?)?;
        let address = endpoint
            .server
            .as_ref()
            .context("test endpoint did not expose a server")?
            .local_addr()?;
        let client = endpoint.client.clone();
        let target = format!("moqt://localhost:{}/secure", address.port()).parse()?;
        let relay = Relay::new(RelayConfig {
            bind: None,
            endpoints: vec![endpoint],
            tls,
            qlog_dir: None,
            mlog_dir: None,
            announce: None,
            node: None,
            coordinator: coordinator.clone(),
            admission,
            development: true,
            listener_security: ListenerSecurityPolicy::Development,
            setup_timeout,
            admission_timeout,
            cleanup_timeout: Duration::from_millis(200),
            session_close_timeout: Duration::from_secs(5),
            max_pending_admissions,
            max_active_sessions,
            token_revalidation_interval: Duration::from_millis(50),
            capacity_limits: RelayCapacityLimits::default(),
            remote_limits: RemoteManagerLimits::default(),
            tracks_limits: moq_transport::serve::TracksLimits::default(),
            request_limits: moq_transport::session::RequestLimits::default(),
        })?;
        let shutdown = CancellationToken::new();
        Ok(RunningRelay {
            client,
            target,
            task: tokio::spawn(relay.run_until(shutdown.clone())),
            shutdown,
            coordinator,
        })
    }

    fn transport_termination_code(error: &web_transport::Error) -> Option<u32> {
        fn session_code(error: &web_transport::quinn::SessionError) -> Option<u32> {
            match error {
                web_transport::quinn::SessionError::WebTransportError(
                    web_transport::quinn::WebTransportError::Closed(code, _),
                ) => Some(*code),
                web_transport::quinn::SessionError::ConnectionError(
                    web_transport::quinn::quinn::ConnectionError::ApplicationClosed(close),
                ) => {
                    let code = close.error_code.into_inner();
                    web_transport::quinn::proto::error_from_http3(code)
                        .or_else(|| u32::try_from(code).ok())
                }
                _ => None,
            }
        }

        match error {
            web_transport::Error::Session(error) => session_code(error),
            web_transport::Error::Read(web_transport::quinn::ReadError::SessionError(error))
            | web_transport::Error::Write(web_transport::quinn::WriteError::SessionError(error)) => {
                session_code(error)
            }
            _ => None,
        }
    }

    fn session_termination_code(error: &moq_transport::session::SessionError) -> Option<u32> {
        match error {
            moq_transport::session::SessionError::WebTransport(error) => {
                transport_termination_code(error)
            }
            _ => None,
        }
    }

    async fn expect_rejected_with_setup(
        client: &quic::Client,
        target: &moq_transport::session::SessionTarget,
        expected_code: moq_transport::session::SessionTerminationCode,
    ) -> anyhow::Result<()> {
        expect_rejected_with_setup_policy(
            client,
            target,
            quic::SubstratePolicy::RawQuic,
            expected_code,
        )
        .await
    }

    async fn expect_rejected_with_setup_policy(
        client: &quic::Client,
        target: &moq_transport::session::SessionTarget,
        policy: quic::SubstratePolicy,
        expected_code: moq_transport::session::SessionTerminationCode,
    ) -> anyhow::Result<()> {
        expect_rejected_with_authorization_policy(
            client,
            target,
            policy,
            Some(SetupAuthorization::new(b"test-token")?),
            expected_code,
        )
        .await
    }

    async fn expect_rejected_without_setup_authorization_policy(
        client: &quic::Client,
        target: &moq_transport::session::SessionTarget,
        policy: quic::SubstratePolicy,
        expected_code: moq_transport::session::SessionTerminationCode,
    ) -> anyhow::Result<()> {
        expect_rejected_with_authorization_policy(client, target, policy, None, expected_code).await
    }

    async fn expect_rejected_with_authorization_policy(
        client: &quic::Client,
        target: &moq_transport::session::SessionTarget,
        policy: quic::SubstratePolicy,
        authorization: Option<SetupAuthorization>,
        expected_code: moq_transport::session::SessionTerminationCode,
    ) -> anyhow::Result<()> {
        let connection = client.connect_target(target, policy, None).await?;
        let raw = connection.session.clone();
        let setup = moq_transport::session::Session::connect_with_authorization(
            connection.session,
            None,
            connection.negotiated,
            authorization,
        )
        .await;
        // Session framing may observe EOF before it observes the transport's
        // application-close code. In either ordering the session must fail,
        // and the raw transport assertion below is the authoritative check
        // for the exact server termination code.
        match setup {
            Ok((session, _publisher, _subscriber)) => {
                let result = tokio::time::timeout(Duration::from_secs(2), session.run())
                    .await
                    .context("rejected session did not terminate")?;
                let Err(error) = result else {
                    anyhow::bail!("rejected session ended without the server termination error");
                };
                if let Some(code) = session_termination_code(&error) {
                    anyhow::ensure!(
                        code == expected_code.as_u32(),
                        "unexpected rejected-session termination: {error:?}"
                    );
                }
            }
            Err(error) => {
                if let Some(code) = session_termination_code(&error) {
                    anyhow::ensure!(
                        code == expected_code.as_u32(),
                        "unexpected SETUP rejection: {error:?}"
                    );
                }
            }
        }

        let raw_error = tokio::time::timeout(Duration::from_secs(2), raw.closed())
            .await
            .context("server did not close the rejected transport")?;
        anyhow::ensure!(
            transport_termination_code(&raw_error) == Some(expected_code.as_u32()),
            "unexpected transport termination: {raw_error:?}"
        );
        Ok(())
    }

    async fn establish_with_setup(
        client: &quic::Client,
        target: &moq_transport::session::SessionTarget,
    ) -> anyhow::Result<(
        web_transport::Session,
        tokio::task::JoinHandle<Result<(), moq_transport::session::SessionError>>,
    )> {
        establish_with_setup_policy(client, target, quic::SubstratePolicy::RawQuic).await
    }

    async fn establish_with_setup_policy(
        client: &quic::Client,
        target: &moq_transport::session::SessionTarget,
        policy: quic::SubstratePolicy,
    ) -> anyhow::Result<(
        web_transport::Session,
        tokio::task::JoinHandle<Result<(), moq_transport::session::SessionError>>,
    )> {
        let connection = client.connect_target(target, policy, None).await?;
        let raw = connection.session.clone();
        let (session, _publisher, _subscriber) =
            moq_transport::session::Session::connect_with_authorization(
                connection.session,
                None,
                connection.negotiated,
                Some(SetupAuthorization::new(b"test-token")?),
            )
            .await?;
        Ok((raw, tokio::spawn(session.run())))
    }

    async fn close_established_session(
        raw: web_transport::Session,
        task: tokio::task::JoinHandle<Result<(), moq_transport::session::SessionError>>,
        reason: &'static str,
    ) -> anyhow::Result<()> {
        raw.close(0, reason);
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .context("client session task did not stop after local close")?
            .context("client session task panicked")?;
        let graceful = match &result {
            Ok(()) => true,
            Err(error) => error.is_graceful_close(),
        };
        anyhow::ensure!(graceful, "client session stopped unexpectedly: {result:?}");
        Ok(())
    }

    async fn expect_session_task_termination(
        task: tokio::task::JoinHandle<Result<(), moq_transport::session::SessionError>>,
        expected_code: moq_transport::session::SessionTerminationCode,
    ) -> anyhow::Result<()> {
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .context("client session task did not observe server termination")?
            .context("client session task panicked")?;
        let Err(error) = result else {
            anyhow::bail!("client session ended without the server termination error");
        };
        anyhow::ensure!(
            session_termination_code(&error) == Some(expected_code.as_u32()),
            "unexpected client session termination: {error:?}"
        );
        Ok(())
    }

    async fn expect_pre_activation_session_stop(
        task: tokio::task::JoinHandle<Result<(), moq_transport::session::SessionError>>,
        expected_code: moq_transport::session::SessionTerminationCode,
    ) -> anyhow::Result<()> {
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .context("client session task did not stop after pre-activation termination")?
            .context("client session task panicked")?;
        let Err(error) = result else {
            anyhow::bail!("client session ended without the server termination error");
        };
        let termination_observed = session_termination_code(&error) == Some(expected_code.as_u32());
        let close_interrupted_termination_frame = matches!(
            &error,
            moq_transport::session::SessionError::Decode(moq_transport::coding::DecodeError::More(
                _
            ))
        );
        anyhow::ensure!(
            termination_observed || close_interrupted_termination_frame,
            "unexpected client session termination: {error:?}"
        );
        Ok(())
    }

    async fn expect_transport_termination(
        raw: &web_transport::Session,
        expected_code: moq_transport::session::SessionTerminationCode,
    ) -> anyhow::Result<()> {
        let error = tokio::time::timeout(Duration::from_secs(2), raw.closed())
            .await
            .context("transport did not observe server termination")?;
        anyhow::ensure!(
            transport_termination_code(&error) == Some(expected_code.as_u32()),
            "unexpected transport termination: {error:?}"
        );
        Ok(())
    }

    async fn expect_session_task_failure(
        task: tokio::task::JoinHandle<Result<(), moq_transport::session::SessionError>>,
    ) -> anyhow::Result<()> {
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .context("client session task did not stop after server failure")?
            .context("client session task panicked")?;
        anyhow::ensure!(result.is_err(), "failed client session ended successfully");
        Ok(())
    }

    async fn start_production_mtls_relay() -> anyhow::Result<(RunningRelay, quic::Client)> {
        start_production_mtls_relay_with_role(ListenerSecurityPolicy::MutualTlsPublisher).await
    }

    async fn start_production_mtls_relay_with_role(
        listener_security: ListenerSecurityPolicy,
    ) -> anyhow::Result<(RunningRelay, quic::Client)> {
        anyhow::ensure!(
            matches!(
                listener_security,
                ListenerSecurityPolicy::MutualTlsPublisher
                    | ListenerSecurityPolicy::MutualTlsRelaySubscriber
            ),
            "production mTLS helper requires a certificate listener"
        );
        let pki = production_pki()?;
        let tls = tls::Args {
            cert: vec![pki.server_cert.clone()],
            key: vec![pki.server_key.clone()],
            root: vec![pki.ca.clone()],
            client_auth: tls::ClientAuthMode::Required,
            client_ca: vec![pki.ca.clone()],
            ..Default::default()
        }
        .load()?;
        let endpoint = Endpoint::new(quic::Config::new(
            "127.0.0.1:0".parse()?,
            None,
            tls.clone(),
        )?)?;
        let address = endpoint
            .server
            .as_ref()
            .context("test endpoint did not expose a server")?
            .local_addr()?;
        let target = format!("moqt://localhost:{}/secure", address.port()).parse()?;
        let coordinator = Arc::new(CountingCoordinator {
            resolved_scope: Some("/secure"),
            ..Default::default()
        });
        let bindings = [format!("{}=/secure", pki.client_fingerprint)];
        let admission = match listener_security {
            ListenerSecurityPolicy::MutualTlsPublisher => {
                crate::CertificateFingerprintAdmission::new_bindings_with_limit(bindings, 4)?
            }
            ListenerSecurityPolicy::MutualTlsRelaySubscriber => {
                crate::CertificateFingerprintAdmission::new_relay_subscriber_bindings_with_limit(
                    bindings, 4,
                )?
            }
            _ => unreachable!("mTLS helper role was validated above"),
        };
        let relay = Relay::new(RelayConfig {
            bind: None,
            endpoints: vec![endpoint],
            tls,
            qlog_dir: None,
            mlog_dir: None,
            announce: None,
            node: None,
            coordinator: coordinator.clone(),
            admission,
            development: false,
            listener_security,
            setup_timeout: Duration::from_secs(1),
            admission_timeout: Duration::from_secs(1),
            cleanup_timeout: Duration::from_millis(200),
            session_close_timeout: Duration::from_millis(200),
            max_pending_admissions: 4,
            max_active_sessions: 8,
            token_revalidation_interval: Duration::from_secs(1),
            capacity_limits: RelayCapacityLimits::default(),
            remote_limits: RemoteManagerLimits::default(),
            tracks_limits: moq_transport::serve::TracksLimits::default(),
            request_limits: moq_transport::session::RequestLimits::default(),
        })?;

        let client_tls = tls::Args {
            root: vec![pki.ca],
            client_cert: Some(pki.client_cert),
            client_key: Some(pki.client_key),
            ..Default::default()
        }
        .load()?;
        let client =
            Endpoint::new(quic::Config::new("127.0.0.1:0".parse()?, None, client_tls)?)?.client;
        let shutdown = CancellationToken::new();
        Ok((
            RunningRelay {
                client: client.clone(),
                target,
                task: tokio::spawn(relay.run_until(shutdown.clone())),
                shutdown,
                coordinator,
            },
            client,
        ))
    }

    async fn start_production_token_relay(
        admission: Arc<LifecycleAdmission>,
        listener_security: ListenerSecurityPolicy,
    ) -> anyhow::Result<RunningRelay> {
        anyhow::ensure!(
            matches!(
                listener_security,
                ListenerSecurityPolicy::TokenSubscriber
                    | ListenerSecurityPolicy::RawQuicTokenSubscriber
            ),
            "production token relay helper requires a token-subscriber policy"
        );
        let pki = production_pki()?;
        let tls = tls::Args {
            cert: vec![pki.server_cert.clone()],
            key: vec![pki.server_key.clone()],
            root: vec![pki.ca.clone()],
            client_auth: tls::ClientAuthMode::Disabled,
            ..Default::default()
        }
        .load()?;
        let endpoint = Endpoint::new(quic::Config::new(
            "127.0.0.1:0".parse()?,
            None,
            tls.clone(),
        )?)?;
        let address = endpoint
            .server
            .as_ref()
            .context("test endpoint did not expose a server")?
            .local_addr()?;
        let target = format!("moqt://localhost:{}/secure", address.port()).parse()?;
        let coordinator = Arc::new(CountingCoordinator {
            resolved_scope: Some("/secure"),
            ..Default::default()
        });
        let relay = Relay::new(RelayConfig {
            bind: None,
            endpoints: vec![endpoint],
            tls,
            qlog_dir: None,
            mlog_dir: None,
            announce: None,
            node: None,
            coordinator: coordinator.clone(),
            admission,
            development: false,
            listener_security,
            setup_timeout: Duration::from_secs(1),
            admission_timeout: Duration::from_secs(1),
            cleanup_timeout: Duration::from_millis(200),
            session_close_timeout: Duration::from_secs(1),
            max_pending_admissions: 4,
            max_active_sessions: 8,
            token_revalidation_interval: Duration::from_millis(25),
            capacity_limits: RelayCapacityLimits::default(),
            remote_limits: RemoteManagerLimits::default(),
            tracks_limits: moq_transport::serve::TracksLimits::default(),
            request_limits: moq_transport::session::RequestLimits::default(),
        })?;

        let client_tls = tls::Args {
            root: vec![pki.ca],
            ..Default::default()
        }
        .load()?;
        let client =
            Endpoint::new(quic::Config::new("127.0.0.1:0".parse()?, None, client_tls)?)?.client;
        let shutdown = CancellationToken::new();
        Ok(RunningRelay {
            client,
            target,
            task: tokio::spawn(relay.run_until(shutdown.clone())),
            shutdown,
            coordinator,
        })
    }

    async fn connect_production_session(
        client: &quic::Client,
        target: &moq_transport::session::SessionTarget,
        policy: quic::SubstratePolicy,
    ) -> anyhow::Result<(
        web_transport::Session,
        tokio::task::JoinHandle<Result<(), moq_transport::session::SessionError>>,
    )> {
        let connection = client.connect_target(target, policy, None).await?;
        let raw = connection.session.clone();
        let (session, _publisher, _subscriber) = moq_transport::session::Session::connect(
            connection.session,
            None,
            connection.negotiated,
        )
        .await?;
        Ok((raw, tokio::spawn(session.run())))
    }

    async fn connect_production_full_session(
        client: &quic::Client,
        target: &moq_transport::session::SessionTarget,
    ) -> anyhow::Result<(
        web_transport::Session,
        moq_transport::session::Publisher,
        moq_transport::session::Subscriber,
        tokio::task::JoinHandle<Result<(), moq_transport::session::SessionError>>,
    )> {
        let connection = client
            .connect_target(target, quic::SubstratePolicy::RawQuic, None)
            .await?;
        let raw = connection.session.clone();
        let (session, publisher, subscriber) = moq_transport::session::Session::connect(
            connection.session,
            None,
            connection.negotiated,
        )
        .await?;
        Ok((raw, publisher, subscriber, tokio::spawn(session.run())))
    }

    #[tokio::test]
    async fn admission_denial_precedes_coordinator_mutation_and_cleans_up() -> anyhow::Result<()> {
        let admission = RecordingAdmission::deny();
        let relay = start_development_relay(
            admission.clone(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            1,
            100,
        )
        .await?;

        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;

        assert_eq!(admission.calls.load(Ordering::SeqCst), 2);
        assert!(admission.context_valid.load(Ordering::SeqCst));
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 0);
        assert_eq!(relay.coordinator.mutation_calls.load(Ordering::SeqCst), 0);
        assert!(!relay.task.is_finished(), "denial terminated the listener");
        relay.task.abort();
        let _ = relay.task.await;
        Ok(())
    }

    #[tokio::test]
    async fn production_mtls_publisher_is_raw_quic_only_and_scope_bound() -> anyhow::Result<()> {
        let (relay, client) = start_production_mtls_relay().await?;

        let (raw, task) =
            connect_production_session(&client, &relay.target, quic::SubstratePolicy::RawQuic)
                .await?;
        tokio::time::timeout(Duration::from_secs(1), async {
            while relay.coordinator.resolve_calls.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        close_established_session(raw, task, "production mTLS raw QUIC test complete").await?;

        expect_rejected_with_setup_policy(
            &client,
            &relay.target,
            quic::SubstratePolicy::WebTransport,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 1);

        let cross_scope: moq_transport::session::SessionTarget = format!(
            "moqt://localhost:{}/other-tenant",
            relay.target.port().unwrap()
        )
        .parse()?;
        expect_rejected_with_setup(
            &client,
            &cross_scope,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(relay.coordinator.mutation_calls.load(Ordering::SeqCst), 0);

        let (raw, task) =
            connect_production_session(&client, &relay.target, quic::SubstratePolicy::RawQuic)
                .await?;
        wait_for_counter(&relay.coordinator.resolve_calls, 2).await?;
        close_established_session(raw, task, "production mTLS listener health check").await?;

        relay.shutdown.cancel();
        relay.task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn production_mtls_relay_subscriber_can_subscribe_but_cannot_publish(
    ) -> anyhow::Result<()> {
        let (relay, client) =
            start_production_mtls_relay_with_role(ListenerSecurityPolicy::MutualTlsRelaySubscriber)
                .await?;

        let (raw, mut publisher, mut subscriber, task) =
            connect_production_full_session(&client, &relay.target).await?;
        wait_for_counter(&relay.coordinator.resolve_calls, 1).await?;

        let namespace = TrackNamespace::from_utf8_path("tenant/live");
        let (track_writer, _track_reader) =
            moq_transport::serve::Track::new(namespace.clone(), "audio/main").produce();
        // The test coordinator deliberately has no origin. Reaching lookup
        // proves the subscribe request traversed the authorized Producer path
        // rather than the disabled-role rejection loop.
        assert!(subscriber.subscribe_open(track_writer).await.is_err());
        wait_for_counter(&relay.coordinator.lookup_calls, 1).await?;

        let publication = publisher.publish_namespace_open(namespace.clone()).await?;
        assert!(publication
            .accepted_with_timeout(Duration::from_secs(1))
            .await
            .is_err());
        assert_eq!(relay.coordinator.mutation_calls.load(Ordering::SeqCst), 0);

        // Publish rejection is request-scoped: the authenticated subscriber
        // connection remains usable for a later subscription.
        let (track_writer, _track_reader) =
            moq_transport::serve::Track::new(namespace, "catalog").produce();
        assert!(subscriber.subscribe_open(track_writer).await.is_err());
        wait_for_counter(&relay.coordinator.lookup_calls, 2).await?;

        drop(publication);
        drop(publisher);
        drop(subscriber);
        close_established_session(raw, task, "mTLS relay subscriber test complete").await?;

        expect_rejected_without_setup_authorization_policy(
            &client,
            &relay.target,
            quic::SubstratePolicy::WebTransport,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        let cross_scope: moq_transport::session::SessionTarget = format!(
            "moqt://localhost:{}/other-tenant",
            relay.target.port().unwrap()
        )
        .parse()?;
        expect_rejected_without_setup_authorization_policy(
            &client,
            &cross_scope,
            quic::SubstratePolicy::RawQuic,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(relay.coordinator.mutation_calls.load(Ordering::SeqCst), 0);

        relay.shutdown.cancel();
        relay.task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn production_token_subscriber_is_webtransport_only_and_revalidates() -> anyhow::Result<()>
    {
        let admission = LifecycleAdmission::new(LifecycleDecision::ProductionToken);
        let relay = start_production_token_relay(
            admission.clone(),
            ListenerSecurityPolicy::TokenSubscriber,
        )
        .await?;

        // Native subscribers need a separately configured raw-QUIC token
        // listener; the browser listener rejects this substrate before policy
        // state can be mutated.
        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(admission.state.admissions.load(Ordering::SeqCst), 0);

        let (_raw, client_task) = establish_with_setup_policy(
            &relay.client,
            &relay.target,
            quic::SubstratePolicy::WebTransport,
        )
        .await?;
        wait_for_counter(&admission.state.admissions, 1).await?;
        assert_eq!(admission.state.active_leases.load(Ordering::SeqCst), 1);

        admission.state.revalidate_ok.store(false, Ordering::SeqCst);
        wait_for_counter(&admission.state.close_calls, 1).await?;
        wait_for_counter(&admission.state.active_leases, 0).await?;
        expect_session_task_termination(
            client_task,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(
            admission.state.close_reasons.lock().unwrap().as_slice(),
            &[AdmissionCloseReason::AdmissionRevalidationFailed]
        );

        admission.state.revalidate_ok.store(true, Ordering::SeqCst);
        let (raw, task) = establish_with_setup_policy(
            &relay.client,
            &relay.target,
            quic::SubstratePolicy::WebTransport,
        )
        .await?;
        wait_for_counter(&admission.state.admissions, 2).await?;
        close_established_session(raw, task, "production token listener health check").await?;
        wait_for_counter(&admission.state.close_calls, 2).await?;

        relay.shutdown.cancel();
        relay.task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn production_raw_quic_token_subscriber_requires_setup_and_revalidates(
    ) -> anyhow::Result<()> {
        let admission = LifecycleAdmission::new(LifecycleDecision::ProductionToken);
        let relay = start_production_token_relay(
            admission.clone(),
            ListenerSecurityPolicy::RawQuicTokenSubscriber,
        )
        .await?;

        // The native listener rejects WebTransport before the external policy
        // can claim replay state or distributed capacity.
        expect_rejected_with_setup_policy(
            &relay.client,
            &relay.target,
            quic::SubstratePolicy::WebTransport,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(admission.state.admissions.load(Ordering::SeqCst), 0);

        // Raw QUIC still carries authorization in draft-19 SETUP. Anonymous
        // TLS plus a missing SETUP credential is not an admitted listener.
        expect_rejected_without_setup_authorization_policy(
            &relay.client,
            &relay.target,
            quic::SubstratePolicy::RawQuic,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(admission.state.admissions.load(Ordering::SeqCst), 0);
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 0);
        assert_eq!(relay.coordinator.mutation_calls.load(Ordering::SeqCst), 0);

        let (_raw, client_task) = establish_with_setup_policy(
            &relay.client,
            &relay.target,
            quic::SubstratePolicy::RawQuic,
        )
        .await?;
        wait_for_counter(&admission.state.admissions, 1).await?;
        assert_eq!(admission.state.active_leases.load(Ordering::SeqCst), 1);
        assert_eq!(relay.coordinator.mutation_calls.load(Ordering::SeqCst), 0);

        admission.state.revalidate_ok.store(false, Ordering::SeqCst);
        wait_for_counter(&admission.state.close_calls, 1).await?;
        wait_for_counter(&admission.state.active_leases, 0).await?;
        expect_session_task_termination(
            client_task,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(
            admission.state.close_reasons.lock().unwrap().as_slice(),
            &[AdmissionCloseReason::AdmissionRevalidationFailed]
        );

        // A failed lease does not poison the listener; a new token session can
        // be admitted and receives the same bounded, awaited close lifecycle.
        admission.state.revalidate_ok.store(true, Ordering::SeqCst);
        let (raw, task) = establish_with_setup_policy(
            &relay.client,
            &relay.target,
            quic::SubstratePolicy::RawQuic,
        )
        .await?;
        wait_for_counter(&admission.state.admissions, 2).await?;
        close_established_session(raw, task, "production raw QUIC token health check").await?;
        wait_for_counter(&admission.state.close_calls, 2).await?;
        wait_for_counter(&admission.state.active_leases, 0).await?;
        assert_eq!(relay.coordinator.mutation_calls.load(Ordering::SeqCst), 0);

        relay.shutdown.cancel();
        relay.task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn raw_quic_token_subscriber_rejects_publish_claim_before_routing() -> anyhow::Result<()>
    {
        let admission = LifecycleAdmission::new(LifecycleDecision::ProductionPublishingToken);
        let relay = start_production_token_relay(
            admission.clone(),
            ListenerSecurityPolicy::RawQuicTokenSubscriber,
        )
        .await?;

        expect_rejected_with_setup_policy(
            &relay.client,
            &relay.target,
            quic::SubstratePolicy::RawQuic,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        wait_for_counter(&admission.state.admissions, 1).await?;
        wait_for_counter(&admission.state.close_calls, 1).await?;
        wait_for_counter(&admission.state.active_leases, 0).await?;
        assert_eq!(
            admission.state.close_reasons.lock().unwrap().as_slice(),
            &[AdmissionCloseReason::ActivationFailed]
        );
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 0);
        assert_eq!(relay.coordinator.mutation_calls.load(Ordering::SeqCst), 0);

        relay.shutdown.cancel();
        relay.task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn production_token_revalidation_failure_before_activation_still_closes(
    ) -> anyhow::Result<()> {
        let admission = LifecycleAdmission::new(LifecycleDecision::ProductionToken);
        admission.state.revalidate_ok.store(false, Ordering::SeqCst);
        let relay = start_production_token_relay(
            admission.clone(),
            ListenerSecurityPolicy::TokenSubscriber,
        )
        .await?;

        let (raw, client_task) = establish_with_setup_policy(
            &relay.client,
            &relay.target,
            quic::SubstratePolicy::WebTransport,
        )
        .await?;
        wait_for_counter(&admission.state.close_calls, 1).await?;
        wait_for_counter(&admission.state.active_leases, 0).await?;
        expect_transport_termination(
            &raw,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        expect_pre_activation_session_stop(
            client_task,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.state.close_reasons.lock().unwrap().as_slice(),
            &[AdmissionCloseReason::AdmissionRevalidationFailed]
        );

        relay.shutdown.cancel();
        relay.task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn setup_deadline_releases_the_pending_admission_permit() -> anyhow::Result<()> {
        let admission = RecordingAdmission::deny();
        let relay = start_development_relay(
            admission.clone(),
            Duration::from_millis(75),
            Duration::from_secs(1),
            1,
            100,
        )
        .await?;

        let connection = relay
            .client
            .connect_target(&relay.target, quic::SubstratePolicy::RawQuic, None)
            .await?;
        tokio::time::timeout(Duration::from_secs(2), connection.session.closed())
            .await
            .context("SETUP-less connection was not closed")?;

        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(admission.calls.load(Ordering::SeqCst), 1);
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 0);
        relay.task.abort();
        let _ = relay.task.await;
        Ok(())
    }

    #[tokio::test]
    async fn admission_timeout_and_capacity_do_not_leak_slots() -> anyhow::Result<()> {
        let admission = RecordingAdmission::slow(Duration::from_secs(1));
        let relay = start_development_relay(
            admission.clone(),
            Duration::from_secs(1),
            Duration::from_millis(75),
            1,
            100,
        )
        .await?;

        let first_client = relay.client.clone();
        let first_target = relay.target.clone();
        let first = tokio::spawn(async move {
            expect_rejected_with_setup(
                &first_client,
                &first_target,
                moq_transport::session::SessionTerminationCode::Unauthorized,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while admission.calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await?;

        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::InternalError,
        )
        .await?;
        first.await??;
        wait_for_counter(&admission.completions, 1).await?;
        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(admission.calls.load(Ordering::SeqCst), 2);
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 0);
        relay.task.abort();
        let _ = relay.task.await;
        Ok(())
    }

    #[tokio::test]
    async fn admission_timeout_reaps_a_late_grant_exactly_once() -> anyhow::Result<()> {
        let admission = LifecycleAdmission::new(LifecycleDecision::Valid);
        admission.state.grant_blocked.store(true, Ordering::SeqCst);
        let relay = start_development_relay(
            admission.clone(),
            Duration::from_secs(1),
            Duration::from_millis(50),
            2,
            1,
        )
        .await?;

        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        wait_for_counter(&admission.state.admissions, 1).await?;
        assert_eq!(admission.state.active_leases.load(Ordering::SeqCst), 1);
        assert_eq!(admission.state.close_calls.load(Ordering::SeqCst), 0);

        // The late admission still owns the only active slot, so no second
        // policy mutation is possible before the grant is reaped.
        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::InternalError,
        )
        .await?;
        assert_eq!(admission.state.admissions.load(Ordering::SeqCst), 1);

        admission.state.grant_release.add_permits(1);
        wait_for_counter(&admission.state.close_calls, 1).await?;
        wait_for_counter(&admission.state.active_leases, 0).await?;
        assert_eq!(admission.state.dropped_leases.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.state.close_reasons.lock().unwrap().as_slice(),
            &[AdmissionCloseReason::ActivationFailed]
        );

        admission.state.grant_blocked.store(false, Ordering::SeqCst);
        let (raw, task) = establish_with_setup(&relay.client, &relay.target).await?;
        wait_for_counter(&admission.state.admissions, 2).await?;
        close_established_session(raw, task, "late-grant reaper health check").await?;
        wait_for_counter(&admission.state.close_calls, 2).await?;

        relay.shutdown.cancel();
        relay.task.await??;
        assert_eq!(admission.state.close_calls.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[tokio::test]
    async fn forced_relay_task_cancellation_after_grant_runs_the_reaper() -> anyhow::Result<()> {
        let admission = LifecycleAdmission::new(LifecycleDecision::Valid);
        let resolve_started = Arc::new(tokio::sync::Semaphore::new(0));
        let resolve_release = Arc::new(tokio::sync::Semaphore::new(0));
        let coordinator = Arc::new(CountingCoordinator {
            resolve_started: Some(resolve_started.clone()),
            resolve_release: Some(resolve_release),
            ..Default::default()
        });
        let relay = start_development_relay_with_coordinator(
            admission.clone(),
            Duration::from_secs(1),
            Duration::from_secs(5),
            1,
            1,
            coordinator,
        )
        .await?;
        let (raw, client_task) = establish_with_setup(&relay.client, &relay.target).await?;
        wait_for_counter(&admission.state.admissions, 1).await?;
        let started = tokio::time::timeout(Duration::from_secs(2), resolve_started.acquire())
            .await
            .context("scope resolution did not start after grant")?
            .context("scope-resolution barrier closed")?;
        started.forget();

        relay.task.abort();
        let task_error = relay
            .task
            .await
            .expect_err("forced relay cancellation unexpectedly completed");
        assert!(task_error.is_cancelled());
        wait_for_counter(&admission.state.close_calls, 1).await?;
        wait_for_counter(&admission.state.active_leases, 0).await?;
        expect_transport_termination(
            &raw,
            moq_transport::session::SessionTerminationCode::InternalError,
        )
        .await?;
        expect_session_task_failure(client_task).await?;
        assert_eq!(admission.state.dropped_leases.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.state.close_reasons.lock().unwrap().as_slice(),
            &[AdmissionCloseReason::LocalClosed]
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_during_awaited_close_does_not_cancel_the_finalizer() -> anyhow::Result<()>
    {
        let admission = LifecycleAdmission::new(LifecycleDecision::Valid);
        admission.state.close_blocked.store(true, Ordering::SeqCst);
        let relay = start_development_relay(
            admission.clone(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            1,
            1,
        )
        .await?;
        let (raw, client_task) = establish_with_setup(&relay.client, &relay.target).await?;
        wait_for_counter(&admission.state.admissions, 1).await?;
        raw.close(0, "enter blocked admission finalizer");
        wait_for_counter(&admission.state.close_calls, 1).await?;
        assert_eq!(admission.state.active_leases.load(Ordering::SeqCst), 1);

        relay.task.abort();
        let task_error = relay
            .task
            .await
            .expect_err("forced relay cancellation unexpectedly completed");
        assert!(task_error.is_cancelled());
        // The finalizer task owns the lease and both permit resources even
        // after its former connection future is gone.
        assert_eq!(admission.state.active_leases.load(Ordering::SeqCst), 1);
        admission.state.close_release.add_permits(1);
        wait_for_counter(&admission.state.active_leases, 0).await?;
        wait_for_counter(&admission.state.dropped_leases, 1).await?;
        close_established_session(raw, client_task, "finalizer cancellation test complete").await?;
        assert_eq!(admission.state.close_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.state.close_reasons.lock().unwrap().as_slice(),
            &[AdmissionCloseReason::PeerClosed]
        );
        Ok(())
    }

    #[tokio::test]
    async fn forced_panic_after_grant_runs_the_reaper_once() -> anyhow::Result<()> {
        let admission = LifecycleAdmission::new(LifecycleDecision::Valid);
        let resolve_started = Arc::new(tokio::sync::Semaphore::new(0));
        let resolve_release = Arc::new(tokio::sync::Semaphore::new(0));
        let coordinator = Arc::new(CountingCoordinator {
            resolve_started: Some(resolve_started.clone()),
            resolve_release: Some(resolve_release.clone()),
            panic_on_resolve: true,
            ..Default::default()
        });
        let relay = start_development_relay_with_coordinator(
            admission.clone(),
            Duration::from_secs(1),
            Duration::from_secs(5),
            1,
            1,
            coordinator,
        )
        .await?;
        let (raw, client_task) = establish_with_setup(&relay.client, &relay.target).await?;
        wait_for_counter(&admission.state.admissions, 1).await?;
        let started = tokio::time::timeout(Duration::from_secs(2), resolve_started.acquire())
            .await
            .context("scope resolution did not reach the panic barrier")?
            .context("scope-resolution barrier closed")?;
        started.forget();
        resolve_release.add_permits(1);

        let task_error = relay
            .task
            .await
            .expect_err("forced coordinator panic unexpectedly completed");
        assert!(task_error.is_panic());
        wait_for_counter(&admission.state.close_calls, 1).await?;
        wait_for_counter(&admission.state.active_leases, 0).await?;
        expect_transport_termination(
            &raw,
            moq_transport::session::SessionTerminationCode::InternalError,
        )
        .await?;
        expect_session_task_failure(client_task).await?;
        assert_eq!(admission.state.dropped_leases.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.state.close_reasons.lock().unwrap().as_slice(),
            &[AdmissionCloseReason::LocalClosed]
        );
        Ok(())
    }

    #[tokio::test]
    async fn overload_flood_is_rejected_inline_and_listener_recovers() -> anyhow::Result<()> {
        let admission = RecordingAdmission::slow(Duration::from_secs(1));
        let relay = start_development_relay(
            admission.clone(),
            Duration::from_secs(1),
            Duration::from_millis(500),
            1,
            100,
        )
        .await?;

        let first_client = relay.client.clone();
        let first_target = relay.target.clone();
        let first = tokio::spawn(async move {
            expect_rejected_with_setup(
                &first_client,
                &first_target,
                moq_transport::session::SessionTerminationCode::Unauthorized,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while admission.calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await?;

        let flood = (0..32).map(|_| {
            let client = relay.client.clone();
            let target = relay.target.clone();
            tokio::spawn(async move {
                expect_rejected_with_setup(
                    &client,
                    &target,
                    moq_transport::session::SessionTerminationCode::InternalError,
                )
                .await
            })
        });
        let results =
            tokio::time::timeout(Duration::from_secs(3), futures::future::join_all(flood))
                .await
                .context("overload flood retained unbounded cleanup work")?;
        for result in results {
            result??;
        }
        first.await??;
        assert_eq!(admission.calls.load(Ordering::SeqCst), 1);
        wait_for_counter(&admission.completions, 1).await?;

        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(admission.calls.load(Ordering::SeqCst), 2);
        assert!(!relay.task.is_finished());
        relay.task.abort();
        let _ = relay.task.await;
        Ok(())
    }

    #[tokio::test]
    async fn scoped_admission_rejects_unknown_coordinator_scope() -> anyhow::Result<()> {
        let relay = start_development_relay(
            Arc::new(ScopedDevelopmentAdmission),
            Duration::from_secs(1),
            Duration::from_secs(1),
            1,
            10,
        )
        .await?;

        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::Unauthorized,
        )
        .await?;
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(relay.coordinator.mutation_calls.load(Ordering::SeqCst), 0);
        assert!(!relay.task.is_finished());
        relay.task.abort();
        let _ = relay.task.await;
        Ok(())
    }

    #[tokio::test]
    async fn scoped_admission_rejects_malformed_coordinator_scope() -> anyhow::Result<()> {
        assert!(!resolved_scope_is_valid(
            &"x".repeat(AdmissionClaims::MAX_SCOPE_BYTES + 1)
        ));
        for malformed in ["", "tenant\nsmuggled"] {
            let coordinator = Arc::new(CountingCoordinator {
                resolved_scope: Some(malformed),
                ..Default::default()
            });
            let relay = start_development_relay_with_coordinator(
                Arc::new(ScopedDevelopmentAdmission),
                Duration::from_secs(1),
                Duration::from_secs(1),
                1,
                10,
                coordinator,
            )
            .await?;
            expect_rejected_with_setup(
                &relay.client,
                &relay.target,
                moq_transport::session::SessionTerminationCode::Unauthorized,
            )
            .await?;
            assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 1);
            assert_eq!(relay.coordinator.mutation_calls.load(Ordering::SeqCst), 0);
            relay.task.abort();
            let _ = relay.task.await;
        }
        Ok(())
    }

    #[tokio::test]
    async fn active_capacity_and_policy_leases_release_for_reconnect() -> anyhow::Result<()> {
        let admission = CapacityAdmission::new();
        let relay = start_development_relay(
            admission.clone(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            4,
            1,
        )
        .await?;

        let (first_raw, first_task) = establish_with_setup(&relay.client, &relay.target).await?;
        tokio::time::timeout(Duration::from_secs(1), async {
            while admission.leases.load(Ordering::SeqCst) != 1
                || relay.coordinator.resolve_calls.load(Ordering::SeqCst) != 1
            {
                tokio::task::yield_now().await;
            }
        })
        .await?;

        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::InternalError,
        )
        .await?;
        assert_eq!(admission.leases.load(Ordering::SeqCst), 1);
        assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 1);

        first_raw.close(0, "capacity test reconnect");
        let _ = tokio::time::timeout(Duration::from_secs(2), first_task).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while admission.leases.load(Ordering::SeqCst) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .context("policy capacity lease was not released")?;

        let (third_raw, third_task) = establish_with_setup(&relay.client, &relay.target).await?;
        tokio::time::timeout(Duration::from_secs(1), async {
            while admission.leases.load(Ordering::SeqCst) != 1
                || relay.coordinator.resolve_calls.load(Ordering::SeqCst) != 2
            {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        third_raw.close(0, "capacity test complete");
        let _ = tokio::time::timeout(Duration::from_secs(2), third_task).await;

        // The process slot is reserved before external admission, so the
        // capacity-rejected middle connection never invokes policy mutation.
        assert_eq!(admission.calls.load(Ordering::SeqCst), 2);
        relay.task.abort();
        let _ = relay.task.await;
        Ok(())
    }

    #[tokio::test]
    async fn awaited_close_holds_capacity_and_server_ids_change_on_reconnect() -> anyhow::Result<()>
    {
        let admission = LifecycleAdmission::new(LifecycleDecision::Valid);
        admission.state.close_blocked.store(true, Ordering::SeqCst);
        let relay = start_development_relay(
            admission.clone(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            4,
            1,
        )
        .await?;

        let (first_raw, first_task) = establish_with_setup(&relay.client, &relay.target).await?;
        wait_for_counter(&admission.state.admissions, 1).await?;
        assert_eq!(admission.state.active_leases.load(Ordering::SeqCst), 1);
        first_raw.close(0, "awaited close barrier");
        let _ = tokio::time::timeout(Duration::from_secs(2), first_task).await;
        wait_for_counter(&admission.state.close_calls, 1).await?;

        // The relay-global slot remains held until the policy close hook
        // completes, so reconnect cannot create a second external lease.
        expect_rejected_with_setup(
            &relay.client,
            &relay.target,
            moq_transport::session::SessionTerminationCode::InternalError,
        )
        .await?;
        assert_eq!(admission.state.admissions.load(Ordering::SeqCst), 1);
        assert_eq!(admission.state.active_leases.load(Ordering::SeqCst), 1);

        admission.state.close_release.add_permits(1);
        wait_for_counter(&admission.state.active_leases, 0).await?;

        let (second_raw, second_task) = establish_with_setup(&relay.client, &relay.target).await?;
        wait_for_counter(&admission.state.admissions, 2).await?;
        let session_ids = admission.state.session_ids.lock().unwrap().clone();
        assert_eq!(session_ids.len(), 2);
        assert_ne!(session_ids[0], session_ids[1]);

        second_raw.close(0, "awaited close complete");
        let _ = tokio::time::timeout(Duration::from_secs(2), second_task).await;
        wait_for_counter(&admission.state.close_calls, 2).await?;
        admission.state.close_release.add_permits(1);
        wait_for_counter(&admission.state.active_leases, 0).await?;
        assert_eq!(admission.state.dropped_leases.load(Ordering::SeqCst), 2);
        assert_eq!(
            admission.state.close_reasons.lock().unwrap().as_slice(),
            &[
                AdmissionCloseReason::PeerClosed,
                AdmissionCloseReason::PeerClosed
            ]
        );

        relay.shutdown.cancel();
        relay.task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn every_post_grant_activation_rejection_awaits_close() -> anyhow::Result<()> {
        for decision in [
            LifecycleDecision::ScopedUnknown,
            LifecycleDecision::InvalidRole,
        ] {
            let admission = LifecycleAdmission::new(decision);
            let relay = start_development_relay(
                admission.clone(),
                Duration::from_secs(1),
                Duration::from_secs(1),
                1,
                1,
            )
            .await?;

            expect_rejected_with_setup(
                &relay.client,
                &relay.target,
                moq_transport::session::SessionTerminationCode::Unauthorized,
            )
            .await?;
            wait_for_counter(&admission.state.close_calls, 1).await?;
            wait_for_counter(&admission.state.active_leases, 0).await?;
            assert_eq!(
                admission.state.close_reasons.lock().unwrap().as_slice(),
                &[AdmissionCloseReason::ActivationFailed]
            );
            if matches!(decision, LifecycleDecision::InvalidRole) {
                assert_eq!(relay.coordinator.resolve_calls.load(Ordering::SeqCst), 0);
            }

            relay.shutdown.cancel();
            relay.task.await??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn close_error_timeout_and_idempotence_remain_fail_closed() {
        let error_policy = LifecycleAdmission::new(LifecycleDecision::Valid);
        error_policy.state.close_error.store(true, Ordering::SeqCst);
        let mut error_admitted = lifecycle_admitted(&error_policy).await;
        assert_eq!(
            close_admission_lease(
                &mut error_admitted,
                AdmissionCloseReason::ProtocolError,
                Duration::from_secs(1),
            )
            .await,
            AdmissionFinalization::CloseError
        );
        assert_eq!(error_policy.state.close_calls.load(Ordering::SeqCst), 1);
        // The relay still owns the fail-closed lease until the finalizer returns.
        assert_eq!(error_policy.state.active_leases.load(Ordering::SeqCst), 1);
        drop(error_admitted);
        assert_eq!(error_policy.state.active_leases.load(Ordering::SeqCst), 0);

        let timeout_policy = LifecycleAdmission::new(LifecycleDecision::Valid);
        timeout_policy
            .state
            .close_blocked
            .store(true, Ordering::SeqCst);
        let mut timeout_admitted = lifecycle_admitted(&timeout_policy).await;
        assert_eq!(
            close_admission_lease(
                &mut timeout_admitted,
                AdmissionCloseReason::LocalClosed,
                Duration::from_millis(10),
            )
            .await,
            AdmissionFinalization::CloseTimeout
        );
        assert_eq!(timeout_policy.state.close_calls.load(Ordering::SeqCst), 1);
        assert_eq!(timeout_policy.state.active_leases.load(Ordering::SeqCst), 1);
        drop(timeout_admitted);
        assert_eq!(timeout_policy.state.active_leases.load(Ordering::SeqCst), 0);

        let idempotent_policy = LifecycleAdmission::new(LifecycleDecision::Valid);
        let mut idempotent = lifecycle_admitted(&idempotent_policy).await;
        let context = AdmissionCloseContext {
            reason: AdmissionCloseReason::LocalClosed,
            ended_at_unix_seconds: unix_now(),
        };
        idempotent.close(context).await.unwrap();
        idempotent.close(context).await.unwrap();
        assert_eq!(
            idempotent_policy.state.close_calls.load(Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn failed_or_cancelled_replay_close_never_reopens_the_credential() {
        let error_store = FakeReplayStore::new(FakeReplayCloseMode::Error);
        let error_admission = FakeReplayAdmission {
            store: error_store.clone(),
        };
        let error_session = AdmissionSessionId::new("error-session").unwrap();
        let mut error_grant =
            fake_replay_admit(&error_admission, &error_session, b"error-credential")
                .await
                .unwrap();
        assert_eq!(
            close_admission_lease(
                &mut error_grant,
                AdmissionCloseReason::ProtocolError,
                Duration::from_secs(1),
            )
            .await,
            AdmissionFinalization::CloseError
        );
        drop(error_grant);
        assert!(
            fake_replay_admit(&error_admission, &error_session, b"error-credential")
                .await
                .is_err()
        );
        let distinct_error_session = AdmissionSessionId::new("error-distinct-session").unwrap();
        let mut distinct_error = fake_replay_admit(
            &error_admission,
            &distinct_error_session,
            b"error-distinct-credential",
        )
        .await
        .unwrap();
        error_store.set_mode(FakeReplayCloseMode::Success);
        assert_eq!(
            close_admission_lease(
                &mut distinct_error,
                AdmissionCloseReason::PeerClosed,
                Duration::from_secs(1),
            )
            .await,
            AdmissionFinalization::Closed
        );

        let timeout_store = FakeReplayStore::new(FakeReplayCloseMode::Block);
        let timeout_admission = FakeReplayAdmission {
            store: timeout_store.clone(),
        };
        let timeout_session = AdmissionSessionId::new("timeout-session").unwrap();
        let mut timeout_grant =
            fake_replay_admit(&timeout_admission, &timeout_session, b"timeout-credential")
                .await
                .unwrap();
        assert_eq!(
            close_admission_lease(
                &mut timeout_grant,
                AdmissionCloseReason::LocalClosed,
                Duration::from_millis(10),
            )
            .await,
            AdmissionFinalization::CloseTimeout
        );
        drop(timeout_grant);
        assert!(
            fake_replay_admit(&timeout_admission, &timeout_session, b"timeout-credential")
                .await
                .is_err()
        );
        let different_session = AdmissionSessionId::new("timeout-retry-session").unwrap();
        assert!(fake_replay_admit(
            &timeout_admission,
            &different_session,
            b"timeout-credential"
        )
        .await
        .is_err());
        let distinct_timeout_session = AdmissionSessionId::new("timeout-distinct-session").unwrap();
        let mut distinct_timeout = fake_replay_admit(
            &timeout_admission,
            &distinct_timeout_session,
            b"timeout-distinct-credential",
        )
        .await
        .unwrap();
        timeout_store.set_mode(FakeReplayCloseMode::Success);
        assert_eq!(
            close_admission_lease(
                &mut distinct_timeout,
                AdmissionCloseReason::PeerClosed,
                Duration::from_secs(1),
            )
            .await,
            AdmissionFinalization::Closed
        );
        assert_eq!(error_store.close_calls.load(Ordering::SeqCst), 2);
        assert_eq!(timeout_store.close_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn graceful_relay_drain_awaits_admission_close() -> anyhow::Result<()> {
        let admission = LifecycleAdmission::new(LifecycleDecision::Valid);
        let relay = start_development_relay(
            admission.clone(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            2,
            2,
        )
        .await?;
        let (_raw, client_task) = establish_with_setup(&relay.client, &relay.target).await?;
        wait_for_counter(&admission.state.admissions, 1).await?;

        relay.shutdown.cancel();
        relay.task.await??;
        let _ = tokio::time::timeout(Duration::from_secs(2), client_task).await;
        assert_eq!(admission.state.close_calls.load(Ordering::SeqCst), 1);
        assert_eq!(admission.state.active_leases.load(Ordering::SeqCst), 0);
        assert_eq!(
            admission.state.close_reasons.lock().unwrap().as_slice(),
            &[AdmissionCloseReason::RelayShutdown]
        );
        Ok(())
    }

    #[tokio::test]
    async fn close_reason_contract_is_complete() {
        for reason in [
            AdmissionCloseReason::PeerClosed,
            AdmissionCloseReason::LocalClosed,
            AdmissionCloseReason::ActivationFailed,
            AdmissionCloseReason::AdmissionRevalidationFailed,
            AdmissionCloseReason::ProtocolError,
            AdmissionCloseReason::RelayShutdown,
        ] {
            let policy = LifecycleAdmission::new(LifecycleDecision::Valid);
            let mut admitted = lifecycle_admitted(&policy).await;
            assert_eq!(
                close_admission_lease(&mut admitted, reason, Duration::from_secs(1)).await,
                AdmissionFinalization::Closed
            );
            assert_eq!(policy.state.close_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                policy.state.close_reasons.lock().unwrap().as_slice(),
                &[reason]
            );
        }
    }

    #[tokio::test]
    async fn production_rejects_disable_verify_optional_mtls_and_development_allow_all() {
        fn config(
            tls: tls::Config,
            admission: Arc<dyn SessionAdmission>,
            listener_security: ListenerSecurityPolicy,
        ) -> RelayConfig {
            RelayConfig {
                bind: Some("127.0.0.1:0".parse().unwrap()),
                endpoints: Vec::new(),
                tls,
                qlog_dir: None,
                mlog_dir: None,
                announce: None,
                node: None,
                coordinator: Arc::new(CountingCoordinator::default()),
                admission,
                development: false,
                listener_security,
                setup_timeout: Duration::from_secs(1),
                admission_timeout: Duration::from_secs(1),
                cleanup_timeout: Duration::from_millis(100),
                session_close_timeout: Duration::from_millis(100),
                max_pending_admissions: 1,
                max_active_sessions: 1,
                token_revalidation_interval: Duration::from_secs(1),
                capacity_limits: RelayCapacityLimits::default(),
                remote_limits: RemoteManagerLimits::default(),
                tracks_limits: moq_transport::serve::TracksLimits::default(),
                request_limits: moq_transport::session::RequestLimits::default(),
            }
        }

        let mut zero_close_timeout = config(
            development_tls().unwrap(),
            Arc::new(crate::DenyAllAdmission),
            ListenerSecurityPolicy::MutualTlsPublisher,
        );
        zero_close_timeout.session_close_timeout = Duration::ZERO;
        let error = Relay::new(zero_close_timeout)
            .err()
            .expect("zero admission close timeout must fail");
        assert!(error.to_string().contains("session-close timeout"));

        for listener_security in [
            ListenerSecurityPolicy::TokenSubscriber,
            ListenerSecurityPolicy::RawQuicTokenSubscriber,
        ] {
            let no_token_lifecycle = config(
                development_tls().unwrap(),
                Arc::new(crate::DenyAllAdmission),
                listener_security,
            );
            let error = Relay::new(no_token_lifecycle)
                .err()
                .expect("token admission without production leases must fail");
            assert!(error.to_string().contains("replay- and lease-aware"));

            let legacy_token = config(
                development_tls().unwrap(),
                Arc::new(LeaseAdmission::default()),
                listener_security,
            );
            let error = Relay::new(legacy_token)
                .err()
                .expect("legacy split token admission must fail");
            assert!(error.to_string().contains("atomic admission"));

            let atomic_without_close = config(
                development_tls().unwrap(),
                Arc::new(AtomicOnlyAdmission),
                listener_security,
            );
            let error = Relay::new(atomic_without_close)
                .err()
                .expect("token admission without awaited close must fail");
            assert!(error.to_string().contains("awaited replay tombstoning"));
        }

        let mut diagnostics = config(
            development_tls().unwrap(),
            Arc::new(crate::DenyAllAdmission),
            ListenerSecurityPolicy::MutualTlsPublisher,
        );
        diagnostics.mlog_dir = Some(PathBuf::from("."));
        let error = Relay::new(diagnostics)
            .err()
            .expect("production per-session diagnostics must fail");
        assert!(error.to_string().contains("development-only"));

        let keylog_tls = development_tls().unwrap();
        let keylog_endpoint = Endpoint::new(
            quic::Config::new("127.0.0.1:0".parse().unwrap(), None, keylog_tls.clone())
                .unwrap()
                .with_tls_key_log(true),
        )
        .unwrap();
        let mut keylog = config(
            keylog_tls,
            Arc::new(crate::DenyAllAdmission),
            ListenerSecurityPolicy::MutualTlsPublisher,
        );
        keylog.bind = None;
        keylog.endpoints = vec![keylog_endpoint];
        let error = Relay::new(keylog)
            .err()
            .expect("production TLS key logging must fail");
        assert!(error.to_string().contains("TLS key logging"));

        let insecure = development_tls().unwrap();
        let error = Relay::new(config(
            insecure,
            Arc::new(crate::DenyAllAdmission),
            ListenerSecurityPolicy::MutualTlsPublisher,
        ))
        .err()
        .expect("production disable-verify must fail");
        assert!(error.to_string().contains("tls-disable-verify"));

        let cert = fixture("localhost-cert.pem");
        let optional = tls::Args {
            cert: vec![cert.clone()],
            key: vec![fixture("localhost-key.pem")],
            root: vec![cert.clone()],
            client_auth: tls::ClientAuthMode::Optional,
            client_ca: vec![cert],
            ..Default::default()
        }
        .load()
        .unwrap();
        for listener_security in [
            ListenerSecurityPolicy::MutualTlsPublisher,
            ListenerSecurityPolicy::MutualTlsRelaySubscriber,
        ] {
            assert!(Relay::new(config(
                optional.clone(),
                Arc::new(crate::DenyAllAdmission),
                listener_security,
            ))
            .is_err());
        }

        let development = development_tls().unwrap();
        assert!(Relay::new(config(
            development,
            crate::DevelopmentAllowAllAdmission::explicitly_enabled(),
            ListenerSecurityPolicy::Development,
        ))
        .is_err());
    }
}
