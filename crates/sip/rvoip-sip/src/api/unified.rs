//! Lower-level session orchestration API.
//!
//! [`UnifiedCoordinator`] is the shared engine underneath [`StreamPeer`] and
//! [`CallbackPeer`]. It exposes explicit [`SessionId`] values and direct
//! methods for call creation, incoming-call resolution, registration
//! lifecycle management, event subscription, transfer primitives, audio
//! bridging, and media control.
//!
//! Use this module directly when you are building an application framework on
//! top of `rvoip-sip`: B2BUA logic, gateways, carrier-facing services,
//! custom peer abstractions, or multi-leg call orchestration. It is also the
//! surface that exposes deterministic registration shutdown and metadata such
//! as registrar-accepted expiry, refresh timing, Service-Route, and GRUU. For
//! ordinary client/test code, [`StreamPeer`] is usually more ergonomic. For
//! reactive server endpoints, [`CallbackPeer`] is usually the better starting
//! point.
//!
//! Outbound calls flow through one builder, [`UnifiedCoordinator::invite`],
//! with chainable modifiers — `.with_credentials(...)` for per-call digest
//! auth, `.with_pai(...)` for per-call `P-Asserted-Identity`, and
//! `.with_extra_headers(...)` for caller-supplied typed headers on the
//! first INVITE. Terminate the chain with `.send()`.
//!
//! # Example
//!
//! ```rust,no_run
//! use rvoip_sip::{Config, Event, Result, UnifiedCoordinator};
//!
//! # async fn example() -> Result<()> {
//! let coordinator = UnifiedCoordinator::new(Config::local("app", 5060)).await?;
//! let mut events = coordinator.events().await?;
//!
//! let call_id = coordinator
//!     .invite(Some("sip:app@127.0.0.1:5060".to_string()), "sip:bob@127.0.0.1:5070")
//!     .send()
//!     .await?;
//!
//! while let Some(event) = events.next().await {
//!     match event {
//!         Event::CallAnswered { call_id: id, .. } if id == call_id => {
//!             coordinator.send_dtmf(&call_id, '1').await?;
//!             coordinator.hangup(&call_id).await?;
//!         }
//!         Event::CallEnded { call_id: id, .. } if id == call_id => break,
//!         Event::CallFailed { call_id: id, .. } if id == call_id => break,
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`CallbackPeer`]: crate::api::callback_peer::CallbackPeer
//! [`StreamPeer`]: crate::api::stream_peer::StreamPeer

#![deny(missing_docs)]

use crate::adapters::{DialogAdapter, MediaAdapter};
use crate::api::lifecycle::{
    CallLifecycleSnapshot, ExactTerminalClaim, ExactTerminalCompletion, LifecycleIndex,
    SessionEventPublisher,
};
use crate::auth::SipClientAuth;
use crate::errors::{Result, SessionError};
use crate::retained_tasks::RetainedTasks;
use crate::session_lifecycle::{SessionLeaseAuthority, TeardownOutcome};
use crate::session_registry::{PendingInboundBundle, SessionRegistry, SessionRegistryHandle};
use crate::session_store::SessionStore;
use crate::state_machine::{ProcessEventResult, StateMachine, StateMachineHelpers};
use crate::state_table::types::{Action, EventType, Role, SessionId};
use crate::types::CallState;
use crate::types::{IncomingCallInfo, SessionInfo};
// Callback system removed - using event-driven approach
use futures::FutureExt;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use rvoip_media_core::types::AudioFrame;
use rvoip_rtp_core::transport::{
    DEFAULT_RTP_PORT_RANGE_END, DEFAULT_RTP_PORT_RANGE_START, MIN_PORT,
};
use rvoip_sip_core::types::sdp::CryptoSuite;
use rvoip_sip_core::types::{headers::HeaderAccess, headers::HeaderName, Method};
use rvoip_sip_core::{Request, Response};
use std::collections::{BinaryHeap, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};

pub use rvoip_media_core::relay::controller::{
    AudioSource, BridgeError, BridgeHandle, MediaSessionControllerConfig,
};
pub use rvoip_rtp_core::transport::SymmetricRtpPolicy;
pub use rvoip_rtp_core::{RtpSessionBufferConfig, RtpTransportBufferConfig};
pub use rvoip_sip_dialog::api::RelUsage;

const MAX_INBOUND_INVITE_OBSERVERS: usize = 16;
const OUTBOUND_DISPATCH_JOIN_FAILURE: &str = "SIP outbound dispatch task failed (class=join)";

type OutboundDispatchResult =
    std::result::Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>>;

/// Owns a spawned outbound state-machine dispatch until it is joined.
///
/// The state-machine to dialog to transport poll chain is intentionally run
/// from the root of a fresh Tokio task. Keeping this abort-on-drop owner around
/// the join preserves cancellation semantics instead of detaching signaling
/// work when the public builder future is cancelled.
struct AbortOutboundDispatchTaskOnDrop {
    handle: tokio::task::JoinHandle<OutboundDispatchResult>,
    stage_claim: Option<Arc<crate::state_machine::executor::StageDispatchClaim>>,
    armed: bool,
}

#[cfg(test)]
mod guarded_outbound_dispatch_tests {
    use super::{AbortOutboundDispatchTaskOnDrop, OutboundDispatchResult};
    use crate::session_store::SessionState;
    use crate::state_machine::executor::{
        PendingOptionsSlot, ProcessEventResult, StageDispatchClaim,
    };
    use crate::state_table::{Role, SessionId};
    use crate::types::CallState;
    use std::sync::Arc;
    use std::time::Duration;

    fn completed_dispatch() -> OutboundDispatchResult {
        Ok(ProcessEventResult {
            old_state: CallState::Active,
            next_state: None,
            transition: None,
            actions_executed: Vec::new(),
            events_published: Vec::new(),
        })
    }

    fn info_claim(session: &mut SessionState) -> Arc<StageDispatchClaim> {
        let options = Arc::new(rvoip_sip_dialog::api::unified::InfoRequestOptions::default());
        session.pending_info_options = Some(Arc::clone(&options));
        Arc::new(StageDispatchClaim::new(PendingOptionsSlot::Info(options)))
    }

    #[tokio::test]
    async fn dropping_public_dispatch_before_claim_aborts_task() {
        let mut session = SessionState::new(
            SessionId("public-cancel-before-claim".to_string()),
            Role::UAC,
        );
        let claim = info_claim(&mut session);
        let (sent, received) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tokio::task::yield_now().await;
            let _ = sent.send(());
            completed_dispatch()
        });
        let owner = AbortOutboundDispatchTaskOnDrop::with_stage_claim(task, claim);
        drop(owner);

        assert!(
            tokio::time::timeout(Duration::from_secs(1), received)
                .await
                .expect("aborted task retained its sender")
                .is_err(),
            "pre-claim cancellation must abort the spawned dispatch"
        );
    }

    #[tokio::test]
    async fn dropping_public_dispatch_after_claim_detaches_task() {
        let mut session = SessionState::new(
            SessionId("public-cancel-after-claim".to_string()),
            Role::UAC,
        );
        let claim = info_claim(&mut session);
        claim.claim_exact(&mut session).unwrap();
        let (sent, received) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tokio::task::yield_now().await;
            let _ = sent.send(());
            completed_dispatch()
        });
        let owner = AbortOutboundDispatchTaskOnDrop::with_stage_claim(task, claim);
        drop(owner);

        tokio::time::timeout(Duration::from_secs(1), received)
            .await
            .expect("claimed dispatch was aborted")
            .expect("claimed dispatch dropped before its work completed");
    }
}

impl AbortOutboundDispatchTaskOnDrop {
    fn new(handle: tokio::task::JoinHandle<OutboundDispatchResult>) -> Self {
        Self {
            handle,
            stage_claim: None,
            armed: true,
        }
    }

    fn with_stage_claim(
        handle: tokio::task::JoinHandle<OutboundDispatchResult>,
        stage_claim: Arc<crate::state_machine::executor::StageDispatchClaim>,
    ) -> Self {
        Self {
            handle,
            stage_claim: Some(stage_claim),
            armed: true,
        }
    }

    async fn join(mut self) -> std::result::Result<OutboundDispatchResult, tokio::task::JoinError> {
        let result = (&mut self.handle).await;
        self.armed = false;
        result
    }
}

impl Drop for AbortOutboundDispatchTaskOnDrop {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let abort = self
            .stage_claim
            .as_ref()
            .map_or(true, |claim| claim.cancel_before_claim());
        if abort {
            self.handle.abort();
        }
    }
}

/// Authenticated inbound INVITE material exposed only to internal adapter
/// observers before the public `IncomingCall` event is published. `request`
/// is absent when the compatibility event did not retain parseable raw bytes;
/// the authenticated principal is still delivered atomically in that case.
///
/// This deliberately bypasses `SessionRegistry::pending_incoming_request`,
/// whose compatibility slot is not keyed by session and therefore is unsafe
/// for concurrent call routing.
#[derive(Clone)]
pub(crate) struct InboundInviteObservation {
    pub(crate) session_id: SessionId,
    pub(crate) request: Option<Arc<Request>>,
    pub(crate) principal: Option<rvoip_core_traits::identity::AuthenticatedPrincipal>,
}

pub(crate) type InboundInviteObserver =
    Arc<dyn Fn(InboundInviteObservation) + Send + Sync + 'static>;

/// SIP TLS operating mode for signalling transports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SipTlsMode {
    /// Disable SIP TLS transport.
    #[default]
    Disabled,
    /// Dial outbound TLS connections only. This is the normal mode for
    /// registering to an upstream proxy/B2BUA such as Asterisk; no local
    /// certificate/key is required.
    ClientOnly,
    /// Bind a SIP TLS listener only. Requires a local certificate/key.
    ServerOnly,
    /// Bind a listener and support outbound TLS dials. Requires a local
    /// certificate/key for the listener side.
    ClientAndServer,
}

/// How this UA expects SIP peers to reach the Contact it advertises.
///
/// This is intentionally separate from [`SipTlsMode`]. The TLS mode controls
/// sockets; the contact mode controls the SIP registration/routing contract.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SipContactMode {
    /// Advertise a Contact address that peers can dial directly. For SIP TLS
    /// this normally means a local TLS listener and listener certificate/key.
    #[default]
    ReachableContact,
    /// RFC 5626 SIP Outbound: advertise outbound Contact parameters and
    /// receive inbound requests over the registered connection-oriented flow.
    RegisteredFlowRfc5626,
    /// Asterisk/PBX symmetric transport style: keep the registration flow
    /// alive and accept inbound requests on that flow without requiring the
    /// registrar to echo RFC 5626 Contact parameters.
    RegisteredFlowSymmetric,
}

/// Named SRTP suite offer policies for common PBX/carrier interop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpSuitePolicy {
    /// Conservative default: AES-128 CM suites, strongest auth tag first.
    Default,
    /// FreeSWITCH-compatible SDES policy: offer AES-256 CM and AES-128 CM
    /// suites in a deliberate preference order while avoiding AEAD-GCM until
    /// rtp-core supports it end to end.
    FreeSwitchCompatible,
}

impl SrtpSuitePolicy {
    /// Suites to advertise for this policy, in local preference order.
    pub fn suites(self) -> Vec<CryptoSuite> {
        match self {
            Self::Default => vec![
                CryptoSuite::AesCm128HmacSha1_80,
                CryptoSuite::AesCm128HmacSha1_32,
            ],
            Self::FreeSwitchCompatible => vec![
                CryptoSuite::AesCm256HmacSha1_80,
                CryptoSuite::AesCm128HmacSha1_80,
                CryptoSuite::AesCm256HmacSha1_32,
                CryptoSuite::AesCm128HmacSha1_32,
            ],
        }
    }
}

/// Media allocation behavior for SIP sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaMode {
    /// Allocate media-core RTP sessions and RTP ports normally.
    Enabled,
    /// Skip media-core RTP allocation while still emitting SDP.
    ///
    /// The configured `sdp_rtp_port` is advertised unless
    /// [`Config::media_public_addr`] carries a nonzero port, in which case the
    /// explicit public media port is advertised.
    SignalingOnly {
        /// RTP port to advertise in SDP when no public media port override is set.
        sdp_rtp_port: u16,
    },
}

/// SIP media NAT behavior kept as a source-compatible sidecar to [`Config`].
///
/// Existing `Config` struct literals and constructors remain unchanged. Use
/// [`UnifiedCoordinator::new_with_nat`] when overriding the secure default.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SipNatConfig {
    /// Bounded source-learning/rebinding policy for RTP destinations.
    pub symmetric_rtp: SymmetricRtpPolicy,
}

impl SipNatConfig {
    /// Replace the default bounded symmetric-RTP policy.
    pub const fn with_symmetric_rtp_policy(mut self, policy: SymmetricRtpPolicy) -> Self {
        self.symmetric_rtp = policy;
        self
    }

    fn validate(self) -> Result<()> {
        self.symmetric_rtp
            .validate()
            .map_err(|detail| SessionError::ConfigError(detail.to_string()))
    }
}

#[derive(Debug, Clone, Copy)]
enum SetupTeardownTimeoutTerminal {
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupTeardownWatchdogKind {
    AcceptedCall,
    AcceptedCallWithSdp,
    Cancellation,
    OutboundSetup,
    InboundSetup,
}

impl SetupTeardownWatchdogKind {
    fn watched_states(self) -> &'static [CallState] {
        match self {
            Self::AcceptedCall | Self::AcceptedCallWithSdp => {
                &[CallState::Answering, CallState::AnsweringHangupPending]
            }
            Self::Cancellation => &[CallState::CancelPending, CallState::Cancelling],
            Self::OutboundSetup => &[CallState::Initiating],
            Self::InboundSetup => &[
                CallState::Ringing,
                CallState::Answering,
                CallState::AnsweringHangupPending,
            ],
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::AcceptedCall => "UAS accepted call did not receive ACK",
            Self::AcceptedCallWithSdp => "UAS accepted call with SDP did not receive ACK",
            Self::Cancellation => "UAC cancellation did not receive terminal INVITE outcome",
            Self::OutboundSetup => "UAC INVITE did not receive a final setup outcome",
            Self::InboundSetup => "UAS INVITE did not receive a final setup outcome",
        }
    }

    fn terminal(self) -> SetupTeardownTimeoutTerminal {
        match self {
            Self::Cancellation => SetupTeardownTimeoutTerminal::Cancelled,
            Self::AcceptedCall
            | Self::AcceptedCallWithSdp
            | Self::OutboundSetup
            | Self::InboundSetup => SetupTeardownTimeoutTerminal::Failed,
        }
    }
}

/// One compact, generation-qualified timeout record. The old implementation
/// retained a full Tokio task (and a roughly 14 KiB async state machine) for
/// every armed call timeout. Records now share one coordinator scheduler and
/// instantiate the heavy timeout handler only when a deadline actually fires.
struct SetupTeardownDeadline {
    deadline: Instant,
    sequence: u64,
    handle: SessionRegistryHandle,
    entered_state_at: Instant,
    kind: SetupTeardownWatchdogKind,
}

impl PartialEq for SetupTeardownDeadline {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline && self.sequence == other.sequence
    }
}

impl Eq for SetupTeardownDeadline {}

impl PartialOrd for SetupTeardownDeadline {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SetupTeardownDeadline {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap. Reverse both fields so the earliest
        // deadline, then the oldest insertion, is popped first.
        other
            .deadline
            .cmp(&self.deadline)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

struct SetupTeardownDeadlineQueue {
    deadlines: BinaryHeap<SetupTeardownDeadline>,
    next_sequence: u64,
    accepting: bool,
}

impl Default for SetupTeardownDeadlineQueue {
    fn default() -> Self {
        Self {
            deadlines: BinaryHeap::new(),
            next_sequence: 0,
            accepting: true,
        }
    }
}

impl SetupTeardownDeadlineQueue {
    fn push(
        &mut self,
        deadline: Instant,
        handle: SessionRegistryHandle,
        entered_state_at: Instant,
        kind: SetupTeardownWatchdogKind,
    ) -> bool {
        let previous = self.deadlines.peek().map(|entry| entry.deadline);
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.deadlines.push(SetupTeardownDeadline {
            deadline,
            sequence,
            handle,
            entered_state_at,
            kind,
        });
        previous.is_none_or(|previous| deadline < previous)
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.deadlines.peek().map(|entry| entry.deadline)
    }

    fn take_due(&mut self, now: Instant, limit: usize) -> Vec<SetupTeardownDeadline> {
        let mut due = Vec::with_capacity(limit.min(self.deadlines.len()));
        while due.len() < limit
            && self
                .deadlines
                .peek()
                .is_some_and(|entry| entry.deadline <= now)
        {
            due.push(self.deadlines.pop().expect("peeked setup timeout deadline"));
        }
        due
    }

    fn drain(&mut self) -> Vec<SetupTeardownDeadline> {
        self.deadlines.drain().collect()
    }
}

struct SetupTeardownDeadlineScheduler {
    queue: StdMutex<SetupTeardownDeadlineQueue>,
    /// Queue-order changes consumed exclusively by the single heap runner.
    changed: tokio::sync::Notify,
    /// One-shot close wakeup shared by optional long-sleep watchdogs.
    closed: tokio::sync::Notify,
    fire_slots: Arc<tokio::sync::Semaphore>,
    tasks: Arc<RetainedTasks>,
    #[cfg(test)]
    fire_test_gate: SetupTeardownFireTestGate,
    #[cfg(test)]
    runner_waiting_deadline: StdMutex<Option<Instant>>,
    #[cfg(test)]
    runner_waiting_changed: tokio::sync::Notify,
}

#[cfg(test)]
struct SetupTeardownFireTestGate {
    pause_next: std::sync::atomic::AtomicBool,
    started: tokio::sync::Semaphore,
    resume: tokio::sync::Semaphore,
}

#[cfg(test)]
impl Default for SetupTeardownFireTestGate {
    fn default() -> Self {
        Self {
            pause_next: std::sync::atomic::AtomicBool::new(false),
            started: tokio::sync::Semaphore::new(0),
            resume: tokio::sync::Semaphore::new(0),
        }
    }
}

impl Default for SetupTeardownDeadlineScheduler {
    fn default() -> Self {
        Self {
            queue: StdMutex::new(SetupTeardownDeadlineQueue::default()),
            changed: tokio::sync::Notify::new(),
            closed: tokio::sync::Notify::new(),
            fire_slots: Arc::new(tokio::sync::Semaphore::new(
                SETUP_TEARDOWN_TIMEOUT_CONCURRENCY,
            )),
            tasks: RetainedTasks::new(),
            #[cfg(test)]
            fire_test_gate: SetupTeardownFireTestGate::default(),
            #[cfg(test)]
            runner_waiting_deadline: StdMutex::new(None),
            #[cfg(test)]
            runner_waiting_changed: tokio::sync::Notify::new(),
        }
    }
}

impl SetupTeardownDeadlineScheduler {
    fn schedule(
        &self,
        deadline: Instant,
        handle: SessionRegistryHandle,
        entered_state_at: Instant,
        kind: SetupTeardownWatchdogKind,
    ) -> bool {
        let advances_deadline = {
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !queue.accepting {
                return false;
            }
            crate::cleanup_diag::record_setup_teardown_watchdog_armed();
            queue.push(deadline, handle, entered_state_at, kind)
        };
        if advances_deadline {
            self.changed.notify_one();
        }
        true
    }

    fn next_deadline_if_accepting(&self) -> Option<Option<Instant>> {
        let queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.accepting.then(|| queue.next_deadline())
    }

    fn take_due_if_accepting(
        &self,
        now: Instant,
        limit: usize,
    ) -> Option<Vec<SetupTeardownDeadline>> {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.accepting.then(|| queue.take_due(now, limit))
    }

    fn begin_close(&self) {
        {
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue.accepting = false;
        }
        self.tasks.close();
        self.changed.notify_waiters();
        self.closed.notify_waiters();
    }

    /// Admit owner-scoped lifecycle work through the lock-free retained-task
    /// fence. This is used by per-call paths such as hangup and media-idle
    /// supervision, so it must not contend on the deadline heap mutex.
    fn spawn_lifecycle_task(
        &self,
        future: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> bool {
        self.tasks.spawn(future)
    }

    /// Admit a cold deadline-fire child after it has been removed from the
    /// heap. A close racing this admission rejects the child and the runner
    /// accounts the claimed deadline as disarmed.
    fn spawn_deadline_fire(
        &self,
        future: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> bool {
        self.tasks.spawn(future)
    }

    fn is_accepting(&self) -> bool {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .accepting
    }

    async fn sleep_or_closed(&self, duration: Duration) -> bool {
        let sleep = tokio::time::sleep(duration);
        tokio::pin!(sleep);
        let closed = self.closed.notified();
        tokio::pin!(closed);
        closed.as_mut().enable();
        if !self.is_accepting() {
            return false;
        }
        tokio::select! {
            _ = &mut sleep => true,
            _ = &mut closed => false,
        }
    }

    fn drain_queued(&self) -> Vec<SetupTeardownDeadline> {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain()
    }

    async fn close_and_wait(&self, timeout: Duration) -> Result<()> {
        self.begin_close();
        if tokio::time::timeout(timeout, self.tasks.wait_idle())
            .await
            .is_err()
        {
            return Err(SessionError::InternalError(format!(
                "setup/teardown deadline scheduler drain timed out with {} retained tasks",
                self.tasks.count()
            )));
        }
        if self.tasks.panicked() {
            return Err(SessionError::InternalError(
                "setup/teardown deadline scheduler retained task panicked".to_string(),
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn pause_next_fire_for_test(&self) {
        self.fire_test_gate
            .pause_next
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    async fn pause_if_requested_for_test(&self) {
        if !self.fire_test_gate.pause_next.swap(false, Ordering::AcqRel) {
            return;
        }
        self.fire_test_gate.started.add_permits(1);
        let permit = self
            .fire_test_gate
            .resume
            .acquire()
            .await
            .expect("setup/teardown fire test gate remains open");
        permit.forget();
    }

    #[cfg(test)]
    async fn wait_for_paused_fire_for_test(&self) {
        let permit = self
            .fire_test_gate
            .started
            .acquire()
            .await
            .expect("setup/teardown fire test gate remains open");
        permit.forget();
    }

    #[cfg(test)]
    fn resume_paused_fire_for_test(&self) {
        self.fire_test_gate.resume.add_permits(1);
    }

    #[cfg(test)]
    fn is_accepting_for_test(&self) -> bool {
        self.is_accepting()
    }

    #[cfg(test)]
    fn record_runner_waiting_for_test(&self, deadline: Instant) {
        *self
            .runner_waiting_deadline
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(deadline);
        self.runner_waiting_changed.notify_waiters();
    }

    #[cfg(test)]
    async fn wait_for_runner_deadline_for_test(&self, expected: Instant) {
        loop {
            let changed = self.runner_waiting_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if *self
                .runner_waiting_deadline
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                == Some(expected)
            {
                return;
            }
            changed.await;
        }
    }

    #[cfg(any(test, feature = "perf-tests"))]
    fn len(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .deadlines
            .len()
    }
}

const SETUP_TEARDOWN_DEADLINE_BATCH: usize = 2_048;
const SETUP_TEARDOWN_TIMEOUT_CONCURRENCY: usize = 64;
const SETUP_TEARDOWN_SCHEDULER_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const EXACT_RESPONSE_OWNER_DEADLINE: Duration = Duration::from_secs(5);
const EXACT_RESPONSE_RETRY_DELAY: Duration = Duration::from_millis(100);
const EXACT_RESPONSE_SLOW_RETRY_DELAY: Duration = Duration::from_secs(1);
const EXACT_RESPONSE_DEADLINE_BATCH: usize = 2_048;
const EXACT_RESPONSE_DEADLINE_CONCURRENCY: usize = 256;
const EXACT_RESPONSE_SEND_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);
const EXACT_RESPONSE_SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const EXACT_RESPONSE_MAX_RETRIES: u8 = 10;

struct ExactResponseDeadline {
    deadline: Instant,
    sequence: u64,
    transaction: rvoip_sip_dialog::transaction::TransactionKey,
}

impl PartialEq for ExactResponseDeadline {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline && self.sequence == other.sequence
    }
}

impl Eq for ExactResponseDeadline {}

impl PartialOrd for ExactResponseDeadline {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ExactResponseDeadline {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .deadline
            .cmp(&self.deadline)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

struct ExactResponseDeadlineQueue {
    deadlines: BinaryHeap<ExactResponseDeadline>,
    next_sequence: u64,
    accepting: bool,
}

impl Default for ExactResponseDeadlineQueue {
    fn default() -> Self {
        Self {
            deadlines: BinaryHeap::new(),
            next_sequence: 0,
            accepting: true,
        }
    }
}

struct PendingExactResponseRegistry {
    entries: dashmap::DashMap<
        rvoip_sip_dialog::transaction::TransactionKey,
        Arc<crate::api::incoming::ExactResponseObligation>,
    >,
    retry_attempts: dashmap::DashMap<rvoip_sip_dialog::transaction::TransactionKey, u8>,
    deadlines: StdMutex<ExactResponseDeadlineQueue>,
    changed: tokio::sync::Notify,
}

pub(crate) enum ExactResponseRegistration {
    Registered,
    Closed,
    Collision,
}

enum ManagedExactResponseOutcome {
    Completed,
    Busy,
    ZeroWireRetryable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExactResponseRetryCause {
    BusyOrTimeout,
    ZeroWire,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExactResponseRetryPlan {
    delay: Duration,
    slow_path: bool,
}

impl Default for PendingExactResponseRegistry {
    fn default() -> Self {
        Self {
            entries: dashmap::DashMap::new(),
            retry_attempts: dashmap::DashMap::new(),
            deadlines: StdMutex::new(ExactResponseDeadlineQueue::default()),
            changed: tokio::sync::Notify::new(),
        }
    }
}

impl PendingExactResponseRegistry {
    fn register(
        &self,
        obligation: Arc<crate::api::incoming::ExactResponseObligation>,
    ) -> ExactResponseRegistration {
        let transaction = obligation.transaction().clone();
        let mut deadlines = self
            .deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !deadlines.accepting {
            return ExactResponseRegistration::Closed;
        }
        if let Some(existing) = self.entries.get(&transaction) {
            return if Arc::ptr_eq(existing.value(), &obligation) {
                ExactResponseRegistration::Registered
            } else {
                ExactResponseRegistration::Collision
            };
        }
        self.entries.insert(transaction.clone(), obligation);
        let previous = deadlines.deadlines.peek().map(|entry| entry.deadline);
        let deadline = Instant::now() + EXACT_RESPONSE_OWNER_DEADLINE;
        let sequence = deadlines.next_sequence;
        deadlines.next_sequence = deadlines.next_sequence.wrapping_add(1);
        deadlines.deadlines.push(ExactResponseDeadline {
            deadline,
            sequence,
            transaction,
        });
        drop(deadlines);
        if previous.is_none_or(|previous| deadline < previous) {
            self.changed.notify_one();
        }
        ExactResponseRegistration::Registered
    }

    fn remove(&self, transaction: &rvoip_sip_dialog::transaction::TransactionKey) {
        self.entries.remove(transaction);
        self.retry_attempts.remove(transaction);
    }

    fn retry_plan(
        &self,
        transaction: &rvoip_sip_dialog::transaction::TransactionKey,
        cause: ExactResponseRetryCause,
    ) -> ExactResponseRetryPlan {
        if cause == ExactResponseRetryCause::BusyOrTimeout {
            return ExactResponseRetryPlan {
                delay: EXACT_RESPONSE_RETRY_DELAY,
                slow_path: false,
            };
        }

        let mut attempts = self.retry_attempts.entry(transaction.clone()).or_insert(0);
        if *attempts < EXACT_RESPONSE_MAX_RETRIES {
            *attempts += 1;
            ExactResponseRetryPlan {
                delay: EXACT_RESPONSE_RETRY_DELAY,
                slow_path: false,
            }
        } else {
            ExactResponseRetryPlan {
                delay: EXACT_RESPONSE_SLOW_RETRY_DELAY,
                slow_path: true,
            }
        }
    }

    fn reschedule(
        &self,
        transaction: &rvoip_sip_dialog::transaction::TransactionKey,
        delay: Duration,
    ) {
        let mut deadlines = self
            .deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !deadlines.accepting || !self.entries.contains_key(transaction) {
            return;
        }
        let previous = deadlines.deadlines.peek().map(|entry| entry.deadline);
        let deadline = Instant::now() + delay;
        let sequence = deadlines.next_sequence;
        deadlines.next_sequence = deadlines.next_sequence.wrapping_add(1);
        deadlines.deadlines.push(ExactResponseDeadline {
            deadline,
            sequence,
            transaction: transaction.clone(),
        });
        drop(deadlines);
        if previous.is_none_or(|previous| deadline < previous) {
            self.changed.notify_one();
        }
    }

    fn next_deadline_if_accepting(&self) -> Option<Option<Instant>> {
        let deadlines = self
            .deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        deadlines
            .accepting
            .then(|| deadlines.deadlines.peek().map(|entry| entry.deadline))
    }

    fn take_due_if_accepting(
        &self,
        now: Instant,
        limit: usize,
    ) -> Option<Vec<rvoip_sip_dialog::transaction::TransactionKey>> {
        let mut deadlines = self
            .deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !deadlines.accepting {
            return None;
        }
        let mut due = Vec::with_capacity(limit.min(deadlines.deadlines.len()));
        while due.len() < limit
            && deadlines
                .deadlines
                .peek()
                .is_some_and(|entry| entry.deadline <= now)
        {
            let entry = deadlines
                .deadlines
                .pop()
                .expect("peeked exact-response deadline");
            if self.entries.contains_key(&entry.transaction) {
                due.push(entry.transaction);
            }
        }
        Some(due)
    }

    fn obligation(
        &self,
        transaction: &rvoip_sip_dialog::transaction::TransactionKey,
    ) -> Option<Arc<crate::api::incoming::ExactResponseObligation>> {
        self.entries
            .get(transaction)
            .map(|entry| Arc::clone(entry.value()))
    }

    fn snapshot(&self) -> Vec<Arc<crate::api::incoming::ExactResponseObligation>> {
        self.entries
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }

    fn begin_close(&self) {
        self.deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .accepting = false;
        self.changed.notify_waiters();
    }

    fn clear(&self) {
        self.entries.clear();
        self.retry_attempts.clear();
        self.deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .deadlines
            .clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoordinatorShutdownOutcome {
    Succeeded,
    RegistrationRefreshDrainFailed,
    ExactResponseDrainFailed,
    SchedulerDrainFailed,
    DialogStopFailed,
    DriverPanicked,
}

impl CoordinatorShutdownOutcome {
    fn into_result(self) -> Result<()> {
        match self {
            Self::Succeeded => Ok(()),
            Self::RegistrationRefreshDrainFailed => Err(SessionError::InternalError(
                "coordinator shutdown failed (class=registration-refresh-drain)".to_string(),
            )),
            Self::ExactResponseDrainFailed => Err(SessionError::InternalError(
                "coordinator shutdown failed (class=exact-response-drain)".to_string(),
            )),
            Self::SchedulerDrainFailed => Err(SessionError::InternalError(
                "coordinator shutdown failed (class=setup-teardown-scheduler-drain)".to_string(),
            )),
            Self::DialogStopFailed => Err(SessionError::InternalError(
                "coordinator shutdown failed (class=dialog-stop)".to_string(),
            )),
            Self::DriverPanicked => Err(SessionError::InternalError(
                "coordinator shutdown failed (class=driver-panicked)".to_string(),
            )),
        }
    }
}

struct CoordinatorShutdownAttempt {
    outcome: StdMutex<Option<CoordinatorShutdownOutcome>>,
    completed: tokio::sync::Notify,
}

impl CoordinatorShutdownAttempt {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            outcome: StdMutex::new(None),
            completed: tokio::sync::Notify::new(),
        })
    }

    fn outcome(&self) -> Option<CoordinatorShutdownOutcome> {
        *self
            .outcome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn finish(&self, outcome: CoordinatorShutdownOutcome) {
        let mut slot = self
            .outcome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_some() {
            return;
        }
        *slot = Some(outcome);
        drop(slot);
        self.completed.notify_waiters();
    }

    async fn wait(&self) -> CoordinatorShutdownOutcome {
        loop {
            let completed = self.completed.notified();
            tokio::pin!(completed);
            completed.as_mut().enable();
            if let Some(outcome) = self.outcome() {
                return outcome;
            }
            completed.await;
        }
    }
}

#[derive(Default)]
struct CoordinatorShutdownFlights {
    current: StdMutex<Option<Arc<CoordinatorShutdownAttempt>>>,
}

impl CoordinatorShutdownFlights {
    fn begin(&self) -> (Arc<CoordinatorShutdownAttempt>, bool) {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(attempt) = current.as_ref() {
            match attempt.outcome() {
                None | Some(CoordinatorShutdownOutcome::Succeeded) => {
                    return (Arc::clone(attempt), false);
                }
                Some(
                    CoordinatorShutdownOutcome::RegistrationRefreshDrainFailed
                    | CoordinatorShutdownOutcome::ExactResponseDrainFailed
                    | CoordinatorShutdownOutcome::SchedulerDrainFailed
                    | CoordinatorShutdownOutcome::DialogStopFailed
                    | CoordinatorShutdownOutcome::DriverPanicked,
                ) => {}
            }
        }
        let attempt = CoordinatorShutdownAttempt::new();
        *current = Some(Arc::clone(&attempt));
        (attempt, true)
    }
}

struct CoordinatorConstructionGuard {
    coordinator: Option<Arc<UnifiedCoordinator>>,
}

impl CoordinatorConstructionGuard {
    fn new(coordinator: Arc<UnifiedCoordinator>) -> Self {
        Self {
            coordinator: Some(coordinator),
        }
    }

    fn disarm(&mut self) {
        self.coordinator = None;
    }
}

impl Drop for CoordinatorConstructionGuard {
    fn drop(&mut self) {
        if let Some(coordinator) = self.coordinator.take() {
            // Constructor cancellation cannot perform async cleanup in Drop.
            // Start the same cancellation-safe ordered shutdown driver used by
            // public shutdown; its retained Arc owns partial dependencies until
            // cleanup reaches a terminal outcome.
            coordinator.shutdown();
        }
    }
}

#[cfg(test)]
fn exact_terminal_completion_result(completion: ExactTerminalCompletion) -> Result<()> {
    match completion {
        ExactTerminalCompletion::PublishedAndReleased
        | ExactTerminalCompletion::PublicationFailed => Ok(()),
        ExactTerminalCompletion::ReleaseFailed => Err(SessionError::InternalError(
            "exact terminal resource release failed".to_string(),
        )),
        ExactTerminalCompletion::PublicationAndReleaseFailed => Err(SessionError::InternalError(
            "exact terminal publication and resource release failed".to_string(),
        )),
        ExactTerminalCompletion::OwnerDropped => Err(SessionError::InternalError(
            "exact terminal publication owner stopped before completion".to_string(),
        )),
    }
}

/// Resolve an established local-BYE dispatch against its authoritative wire
/// confirmation. A post-send state/lifecycle race may make dispatch report an
/// error even though the adapter retained the exact new transaction. Only
/// that concrete side effect permits joining confirmation; without it the
/// original dispatch error remains authoritative. Confirmation still fails
/// closed on non-2xx, timeout, and an unobservable transaction.
async fn complete_established_bye_dispatch<F>(
    dispatch: Result<()>,
    retained_new_bye: bool,
    confirmation: F,
) -> Result<()>
where
    F: std::future::Future<Output = Result<()>>,
{
    match dispatch {
        Ok(()) => confirmation.await,
        Err(_) if retained_new_bye => confirmation.await,
        Err(error) => Err(error),
    }
}

fn shared_hangup_completion_result(succeeded: bool) -> Result<()> {
    if succeeded {
        Ok(())
    } else {
        Err(SessionError::InvalidTransition(
            "the exact hangup operation failed".to_string(),
        ))
    }
}

struct RetainedHangupTaskCompletion {
    control: Arc<crate::session_store::state::SessionHangupControl>,
    finished: bool,
}

impl RetainedHangupTaskCompletion {
    fn new(control: Arc<crate::session_store::state::SessionHangupControl>) -> Self {
        Self {
            control,
            finished: false,
        }
    }

    fn finish(&mut self, succeeded: bool) {
        self.control.finish(succeeded);
        self.finished = true;
    }
}

impl Drop for RetainedHangupTaskCompletion {
    fn drop(&mut self) {
        if !self.finished {
            self.control.finish(false);
        }
    }
}

/// Runtime configuration for [`UnifiedCoordinator`].
///
/// `Config` controls SIP and media binding, advertised addresses, TLS,
/// registration Contact behavior, registration refresh/unregister policy, SRTP
/// policy, session timers, reliable provisionals, caller identity headers,
/// outbound proxy routing for INVITEs and REGISTERs, NAT/media address
/// discovery, and codec negotiation.
///
/// Start with [`Config::local`] for loopback examples, [`Config::on`] for a
/// specific LAN/host address, then adjust the feature-specific fields for the
/// deployment profile. The profile constructors are conservative starting
/// points for common interop targets; they do not imply carrier certification
/// or full RFC 5626 multi-flow behavior.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip::{Config, SipContactMode, SipTlsMode};
///
/// let lan = Config::on("alice", "192.168.1.50".parse().unwrap(), 5060);
/// assert_eq!(lan.local_uri, "sip:alice@192.168.1.50:5060");
///
/// let tls_registered = Config::local("alice", 5060)
///     .tls_registered_flow_symmetric("urn:uuid:00000000-0000-0000-0000-000000000001");
/// assert_eq!(tls_registered.sip_tls_mode, SipTlsMode::ClientOnly);
/// assert_eq!(
///     tls_registered.sip_contact_mode,
///     SipContactMode::RegisteredFlowSymmetric
/// );
/// ```
#[derive(Clone)]
pub struct Config {
    /// Local IP address for media
    pub local_ip: IpAddr,
    /// SIP port
    pub sip_port: u16,
    /// Starting port for media
    pub media_port_start: u16,
    /// Ending port for media
    pub media_port_end: u16,
    /// Requested media port range capacity when configured by capacity.
    ///
    /// `None` means the explicit start/end range is authoritative. When set,
    /// validation checks that the configured range can satisfy the requested
    /// number of RTP ports.
    pub media_port_capacity: Option<usize>,
    /// Bind address for SIP
    pub bind_addr: SocketAddr,
    /// Optional advertised address for SIP Via sent-by and fallback Contact
    /// generation. This is distinct from [`Config::bind_addr`]: bind can be
    /// `0.0.0.0`, while the advertised address must name a concrete interface.
    pub sip_advertised_addr: Option<SocketAddr>,
    /// Optional path to custom state table YAML
    /// Priority: 1) this config path, 2) embedded default
    pub state_table_path: Option<String>,
    /// Local SIP URI (e.g., "sip:alice@127.0.0.1:5060")
    pub local_uri: String,
    /// Policy for RFC 3262 `100rel` reliable provisionals on outgoing INVITE.
    ///
    /// Default is `Supported` — advertise capability without demanding it,
    /// which is the safe setting for interop and unchanged wire behavior.
    /// Set to `Required` when connecting to a carrier that mandates 100rel,
    /// or `NotSupported` to omit the tag entirely.
    pub use_100rel: RelUsage,

    /// Whether the default inbound INVITE path sends automatic `180 Ringing`.
    ///
    /// Default: `true`, which is appropriate for PBX-style ringing behavior
    /// and preserves previous API behavior. Set to `false` for IVR,
    /// contact-center, and auto-answer services that immediately send a final
    /// response and do not need the extra provisional response.
    pub auto_180_ringing: bool,

    /// Whether INVITE server transactions arm the automatic RFC 3261
    /// `100 Trying` timer.
    ///
    /// Default: `true`, which preserves RFC-friendly behavior for ordinary
    /// endpoints. Set to `false` only for fixed fast-answer services that
    /// immediately send a final response and do not need one timer task per
    /// inbound INVITE.
    pub auto_100_trying: bool,

    /// Whether inbound INVITEs are accepted immediately in the session event
    /// path before application callback dispatch.
    ///
    /// Default: `false`, preserving normal app-controlled accept/reject/defer
    /// behavior. Set to `true` only for fixed auto-answer services and
    /// high-CPS benchmarks where the app callback must not sit on the first
    /// final response path.
    pub fast_auto_accept_incoming_calls: bool,

    /// Maximum seconds a locally-initiated setup teardown state may wait for
    /// its matching dialog event before rvoip-sip synthesizes the existing
    /// `DialogTimeout` transition and releases local resources.
    ///
    /// This bounds UAC pre-answer cancellation (`CancelPending`/`Cancelling`)
    /// and UAS accepted-but-unacked calls (`Answering`/
    /// `AnsweringHangupPending`). `0` disables the watchdog. Default: `120`.
    pub setup_teardown_timeout_secs: u64,

    /// Maximum seconds an inbound answered call may remain active without
    /// receiving any RTP packets after answer before rvoip-sip sends local BYE
    /// cleanup and releases the session.
    ///
    /// This is disabled by default because SIP permits long silent calls. It
    /// is intended for high-density auto-answer servers and carrier burst
    /// tests where an answered call that never produces RTP is more likely to
    /// be an abandoned late-setup edge than a valid silent call. `0` disables
    /// the watchdog.
    pub active_call_no_media_timeout_secs: u64,

    /// Maximum seconds an inbound answered call may remain active without RTP
    /// packet-count progress after it has previously received media.
    ///
    /// This is disabled by default because SIP permits silent calls and
    /// application-layer policy varies. It is intended for high-density
    /// auto-answer servers and carrier burst tests where a media-bearing call
    /// that stops receiving RTP and never receives BYE should not retain
    /// server resources indefinitely. `0` disables the watchdog.
    pub active_call_media_idle_timeout_secs: u64,

    /// RFC 4028 `Session-Expires` value in seconds to advertise on outgoing
    /// INVITEs. `None` disables session timers entirely. Common carrier
    /// value is 1800 (30 min).
    pub session_timer_secs: Option<u32>,

    /// Minimum-session-expires (`Min-SE:`) we're willing to accept, in
    /// seconds. Default 90 per RFC 4028 §5.
    pub session_timer_min_se: u32,

    /// Default Digest credentials for UAC 401/407 retry.
    ///
    /// This is the Digest shorthand. When a supported outbound request is
    /// challenged and no per-request auth was supplied, `rvoip-sip` can use
    /// these credentials to compute a Digest response. Use
    /// [`Config::auth`](Self::auth) for Bearer, Basic, AKA, or
    /// multi-challenge negotiation. Default: `None`.
    pub credentials: Option<crate::types::Credentials>,

    /// General UAC auth configuration used for 401/407 retries across
    /// supported SIP access-auth schemes.
    ///
    /// Configure this with [`crate::SipClientAuth`] for Digest, Bearer, Basic,
    /// AKA, or [`crate::SipClientAuth::any`] multi-challenge negotiation.
    /// `credentials` remains the Digest shorthand; precedence is per-request
    /// auth, then this field, then `credentials`.
    pub auth: Option<crate::auth::SipClientAuth>,

    /// Default `P-Asserted-Identity` URI (RFC 3325 §9.1) to attach to every
    /// outgoing INVITE. Carrier trunks (Twilio, Vonage, Bandwidth, most PBX
    /// trunks) require PAI for caller-ID assertion on outbound trunk calls;
    /// without it the call is often hard-rejected or stripped of caller ID.
    /// `None` (the default) suppresses the header entirely. Per-call override
    /// is available via [`UnifiedCoordinator::invite`] + `.with_pai(...)`.
    pub pai_uri: Option<String>,

    /// Optional SIP transport-boundary tracing for diagnostics.
    pub sip_trace: crate::api::events::SipTraceConfig,

    /// SIP_API_DESIGN_2 §12.4 — pluggable trace-output redaction. Every
    /// header value passing through the trace sink is run through the
    /// configured policy before logging; the wire form is unaffected.
    /// `None` selects the production-safe [`DefaultTraceRedactor`]; verbatim
    /// traces require an explicit [`PassthroughRedactor`] development/operator
    /// opt-in. See
    /// [`crate::api::trace_redactor::TraceRedactor`] for the policy
    /// hook contract and [`Config::trace_passthrough_for_development`] for the
    /// explicit unsafe override.
    ///
    /// [`DefaultTraceRedactor`]: crate::api::trace_redactor::DefaultTraceRedactor
    /// [`PassthroughRedactor`]: crate::api::trace_redactor::PassthroughRedactor
    pub trace_redaction: Option<std::sync::Arc<dyn crate::api::trace_redactor::TraceRedactor>>,

    /// Outbound proxy URI (RFC 3261 §8.1.2). When set, a `Route:
    /// <outbound-proxy-uri;lr>` header is pre-loaded as the first Route on
    /// every outgoing INVITE this UA originates, forcing the dialog-
    /// initiating request through the specified proxy. Typical values:
    /// `sip:sbc.example.com;lr`, `sips:sbc.example.com:5061;lr`.
    ///
    /// The URI should carry the `;lr` parameter to signal a loose-routing
    /// proxy (RFC 3261 §16.12.1.1). rvoip-sip does **not** auto-add `;lr`
    /// — set it explicitly in the URI string.
    ///
    /// Applied to outgoing INVITEs and REGISTERs. For REGISTER, the registrar
    /// remains the SIP Request-URI while the network destination is the
    /// outbound proxy and a loose Route header is included. `None` (the
    /// default) suppresses the header entirely. Per-request override is not
    /// yet exposed.
    pub outbound_proxy_uri: Option<String>,

    /// Enable RFC 5626 "SIP Outbound" behaviour on outgoing REGISTERs.
    ///
    /// When `true` and [`Config::sip_instance`] is set, the REGISTER Contact
    /// carries the outbound-aware parameters:
    ///
    /// - `+sip.instance="<urn:...>"` (RFC 5626 §4.1) — UA-stable instance
    ///   URN, so the registrar can associate a binding with a specific
    ///   physical device across flow failures.
    /// - `reg-id=1` (RFC 5626 §4.2) — flow identifier. Multi-flow support
    ///   will bump this; today we always register flow 1.
    /// - The Contact URI gets a `;ob` flag (RFC 5626 §5.4) signalling that
    ///   the UA wants the registrar to preserve the flow association.
    ///
    /// Enable this for carriers and SBCs that assume RFC 5626 — most
    /// modern carrier infra does. Default: `false` (pre-5626 REGISTER
    /// behaviour for backwards compatibility).
    ///
    /// When the registrar echoes the outbound Contact in a 2xx REGISTER,
    /// rvoip-sip-dialog starts CRLFCRLF keep-alive pings on the registration
    /// flow. Flow failure is surfaced back into rvoip-sip so the
    /// registration can be refreshed.
    pub sip_outbound_enabled: bool,

    /// UA-stable instance URN advertised on outbound REGISTERs (RFC 5626
    /// §4.1). Typically a `urn:uuid:<uuid>` generated once per device and
    /// persisted across process restarts. Without this, the registrar
    /// cannot tell a restarted UA apart from a different device with the
    /// same AoR — flow stickiness breaks.
    ///
    /// When [`Config::sip_outbound_enabled`] is `true` and this is `None`,
    /// a warning is logged and outbound-aware parameters are suppressed
    /// on the REGISTER (falling back to pre-5626 behaviour). Callers
    /// SHOULD supply a stable URN explicitly; leaving it `None` is only
    /// appropriate for single-shot dev / lab usage.
    pub sip_instance: Option<String>,

    /// Interval in seconds between RFC 5626 §5.1 CRLFCRLF keep-alive pings
    /// on long-lived TCP / TLS flows. Default 25 s per the RFC
    /// recommendation (ping every 25 s, flow declared dead after 30 s
    /// without a response).
    ///
    /// This is honored when outbound registration flow support is
    /// enabled with a stable [`Config::sip_instance`].
    pub outbound_keepalive_interval_secs: u64,

    /// Automatically refresh successful registrations before they expire.
    ///
    /// When enabled, rvoip-sip schedules a re-REGISTER after a successful
    /// REGISTER 2xx using the registrar-accepted expiry. Default: `true`.
    pub registration_auto_refresh: bool,

    /// Maximum percentage of the refresh interval to subtract as jitter.
    ///
    /// The base refresh interval is 85% of the accepted expiry. Jitter is
    /// applied earlier, never later, so a value of 5 means the refresh fires
    /// between 80.75% and 85% of the accepted expiry. Default: `5`.
    pub registration_refresh_jitter_percent: u8,

    /// Timeout for best-effort unregister during graceful shutdown.
    ///
    /// `0` disables unregister-on-shutdown. Default: `3` seconds.
    pub unregister_on_shutdown_timeout_secs: u64,

    /// SIP TLS signalling mode.
    pub sip_tls_mode: SipTlsMode,

    /// SIP Contact reachability strategy.
    ///
    /// [`SipContactMode::ReachableContact`] is the classic SIP UA model:
    /// the Contact URI is directly reachable by the peer. For SIP TLS that
    /// means this process usually also runs a TLS listener. The registered
    /// flow modes are for proxy/B2BUA deployments where inbound requests are
    /// expected on the existing outbound registration flow.
    pub sip_contact_mode: SipContactMode,

    /// Optional local SIP TLS listener address. Used for
    /// [`SipTlsMode::ServerOnly`] and [`SipTlsMode::ClientAndServer`].
    /// When unset, rvoip-sip-dialog retains its legacy default of deriving the
    /// TLS listener address from [`Config::bind_addr`] by adding 1 to the
    /// port.
    pub tls_bind_addr: Option<SocketAddr>,

    /// Optional advertised address for SIP TLS Via sent-by and fallback
    /// Contact generation. This is distinct from [`Config::tls_bind_addr`]:
    /// bind can be `0.0.0.0`, while the advertised address must be routable
    /// by peers.
    pub tls_advertised_addr: Option<SocketAddr>,

    /// Optional Contact URI override used by rvoip-sip-dialog for
    /// dialog-creating and target-refresh requests. Registrations can
    /// still override Contact per REGISTER via [`Registration`].
    pub contact_uri: Option<String>,

    /// Path to the PEM-encoded TLS listener certificate (RFC 3261
    /// §26.2 / RFC 5630). Required only for [`SipTlsMode::ServerOnly`]
    /// and [`SipTlsMode::ClientAndServer`]. It is not required for
    /// [`SipTlsMode::ClientOnly`], where this endpoint connects to a
    /// remote TLS server and verifies that server's certificate.
    pub tls_cert_path: Option<std::path::PathBuf>,

    /// Path to the PEM-encoded PKCS#8 listener private key matching
    /// [`Config::tls_cert_path`].
    pub tls_key_path: Option<std::path::PathBuf>,

    /// Optional PEM-encoded client certificate chain for mutual TLS.
    /// Leave unset for normal server-authenticated SIP TLS.
    pub tls_client_cert_path: Option<std::path::PathBuf>,

    /// Optional PEM-encoded PKCS#8 private key matching
    /// [`Config::tls_client_cert_path`].
    pub tls_client_key_path: Option<std::path::PathBuf>,

    /// Optional path to a PEM-encoded CA bundle to *add to* the system
    /// trust store on the client side. Used for enterprise PKI / private
    /// carriers where the server cert is signed by a private CA not in
    /// the system root store. Default: `None` (system roots only).
    pub tls_extra_ca_path: Option<std::path::PathBuf>,

    /// Inbound SIP TLS client-certificate verification policy.
    ///
    /// This controls certificates presented by peers connecting to this
    /// listener. It is independent from [`Config::tls_client_cert_path`],
    /// which controls the certificate this endpoint presents on outbound
    /// connections. The default is disabled. Tenant-bound mTLS listener
    /// mappings require `Optional` or `Required` with an explicit client CA.
    pub tls_server_client_auth: rvoip_sip_transport::transport::tls::TlsServerClientAuthConfig,

    /// **Dev only.** When `true`, server certs are accepted without
    /// identity verification. Required for self-signed test certs. The
    /// TLS handshake still runs end-to-end (encrypted), but a malicious
    /// peer can MITM. Default: `false`. **Must not** be enabled in
    /// production.
    ///
    /// Gated behind the `dev-insecure-tls` Cargo feature — production
    /// builds physically cannot access this field. The matching
    /// `InsecureCertVerifier` in `sip-transport` is also feature-gated,
    /// so even with the feature enabled the verifier type only exists
    /// in the dev-build binary.
    #[cfg(feature = "dev-insecure-tls")]
    pub tls_insecure_skip_verify: bool,

    /// Offer RFC 4568 SDES-SRTP on outgoing INVITEs.
    ///
    /// When `true`:
    ///
    /// - The `m=audio` line in the offer uses `RTP/SAVP` (RFC 4568
    ///   §3.1.4) instead of `RTP/AVP`.
    /// - One `a=crypto:` line per suite in
    ///   [`Config::srtp_offered_suites`] is attached, each with a
    ///   freshly-generated master key (RFC 4568 §6.1).
    /// - When the answer accepts SRTP, paired `SrtpContext`s are
    ///   installed on the outgoing+incoming RTP transport before the
    ///   first packet flows. All RTP payload is then AES-encrypted
    ///   end-to-end per RFC 3711.
    ///
    /// Enable this when targeting:
    /// - **Cloud SIP carriers** (Twilio, Vonage, Bandwidth, Telnyx)
    ///   on production tier — they typically require `srtp=mandatory`.
    /// - **Modern Asterisk / FreeSWITCH** trunks configured with
    ///   `srtp=mandatory`.
    /// - **Microsoft Teams Direct Routing** (which also requires TLS
    ///   for signalling — see [`Config::tls_cert_path`]).
    ///
    /// Leave disabled (the default) for:
    /// - LAN-only PBX deployments where carriers don't enforce SRTP.
    /// - Dev / lab setups exercising the RTP path without crypto
    ///   overhead.
    /// - Codec / RTP profile experiments where SRTP would obscure
    ///   the wire bytes.
    ///
    /// See [`Config::srtp_required`] for the strict-mode variant.
    pub offer_srtp: bool,

    /// Refuse to fall back to plaintext RTP when SRTP can't be
    /// negotiated.
    ///
    /// - **UAC**: a remote SDP answer without an acceptable
    ///   `a=crypto:` line causes the call to surface as
    ///   [`Event::CallFailed`](crate::api::events::Event::CallFailed)
    ///   rather than silently downgrading.
    /// - **UAS**: an offer without `a=crypto:` is rejected with
    ///   `488 Not Acceptable Here`.
    ///
    /// Mirrors the RFC 3261 `Require:` header semantic — fail
    /// loudly rather than silently downgrade a security guarantee.
    /// Pair with [`Config::offer_srtp`] = `true` for the canonical
    /// "I require encrypted media" stance.
    ///
    /// Default: `false` — soft-prefer SRTP but accept plaintext.
    pub srtp_required: bool,

    /// SRTP crypto suites to advertise on outgoing offers, in
    /// preference order. The answerer picks the first suite it
    /// supports.
    ///
    /// Default:
    /// `[AesCm128HmacSha1_80, AesCm128HmacSha1_32]` —
    /// RFC 4568 §6.2.1 MTI suite first (`_80`, ubiquitous), then
    /// `_32` (smaller auth tag for bandwidth-conscious carriers).
    /// Modify when a specific carrier requires a non-default
    /// preference.
    pub srtp_offered_suites: Vec<CryptoSuite>,

    /// Override the RTP-side public address advertised in SDP `c=` /
    /// `o=` and `m=audio <port>` lines. Use when:
    ///
    /// - The rvoip-sip process runs behind a 1:1 NAT or IP alias
    ///   and the operator already knows the external IP/port.
    /// - The deployment uses an SBC that performs media latching, and
    ///   we want to advertise the SBC's public IP rather than rely on
    ///   STUN.
    ///
    /// Mutually exclusive with [`Config::stun_server`]. If both are
    /// set, the static override wins and a warning is logged.
    /// Default: `None` — advertise the local interface address (today's
    /// behaviour).
    pub media_public_addr: Option<SocketAddr>,

    /// Media allocation behavior.
    ///
    /// Default: [`MediaMode::Enabled`], which allocates real media-core RTP
    /// sessions and RTP ports. [`MediaMode::SignalingOnly`] skips media-core
    /// RTP allocation but still emits SDP; useful for signaling-only services
    /// and controlled tests.
    pub media_mode: MediaMode,

    /// Optional capacity hint for media-core session and RTP port indexes.
    ///
    /// This is intentionally separate from [`Config::server_call_capacity`]:
    /// high-CPS media servers may want RTP/media preallocation without
    /// inflating SIP dialog and transaction indexes.
    pub media_session_capacity: Option<usize>,

    /// RTP session queue sizing for SIP media calls.
    pub rtp_session_buffer_config: RtpSessionBufferConfig,

    /// RTP transport event and receive buffer sizing for SIP media calls.
    pub rtp_transport_buffer_config: RtpTransportBufferConfig,

    /// Media-core controller pool and capacity tuning for SIP media calls.
    pub media_session_controller_config: MediaSessionControllerConfig,

    /// STUN server (RFC 8489 §14) to probe for the RTP-side public
    /// mapping at coordinator boot. Format: `"host:port"` or `"host"`
    /// (default port 3478). Common public servers:
    /// `stun.l.google.com:19302`, `stun.cloudflare.com:3478`.
    ///
    /// The probe runs once at startup using a fresh UDP socket bound to
    /// [`Config::local_ip`]. This is best-effort address discovery: it is
    /// useful for simple cone-NAT labs, but it does not guarantee the exact
    /// mapping of a later per-call RTP socket. Symmetric NATs and production
    /// Internet edges should use a static [`Config::media_public_addr`] today
    /// or ICE in a future WebRTC/edge layer. Failure mode: probe timeout /
    /// unreachable / unparseable response → log a warning and fall back to
    /// the local interface address. STUN is intentionally soft-fail — the
    /// call path is never blocked on it.
    ///
    /// Default: `None` — no probe runs (today's behaviour).
    pub stun_server: Option<String>,

    /// RFC 3389 Comfort Noise (PT 13) advertisement.
    ///
    /// When `true`, outgoing offers and answers carry `13` in the
    /// `m=audio` format list plus `a=rtpmap:13 CN/8000` so peers know
    /// we accept Comfort Noise during silence periods. The session
    /// also enables media-core comfort-noise support so callers can drive CN
    /// packets through their chosen media-control path.
    ///
    /// Default: `false` — peers see the pre-Sprint-3 PCMU + PCMA +
    /// telephone-event format set with no CN.
    pub comfort_noise_enabled: bool,

    /// RFC 3264 §6 strict codec matching for SDP answers.
    ///
    /// When `true` (default), the SDP answer's format list is the
    /// strict intersection of the offer's formats and our supported
    /// set, in offerer-preference order. RFC-correct: a peer that
    /// offered `0 101` (PCMU + telephone-event only) gets answered
    /// with `0 101`, not `0 8 101`.
    ///
    /// When `false`, the answer always advertises our full supported
    /// set regardless of offer (the pre-Sprint-3.5 permissive
    /// behaviour). Set to `false` for deployments where a carrier or
    /// PBX accidentally relied on the legacy "always full set"
    /// answer shape — this provides a one-line escape hatch back to
    /// the prior behaviour without code changes.
    ///
    /// Default: `true`.
    pub strict_codec_matching: bool,

    /// RTP payload types this UA advertises in outgoing offers and accepts in
    /// answers. Default `[0, 8, 101]` (PCMU + PCMA + telephone-event)
    /// preserves the established beta media profile.
    ///
    /// Beta validation intentionally rejects audio payload types that
    /// media-core cannot encode/decode end to end. The advertised full-media
    /// set is limited to PCMU (`0`), PCMA (`8`), telephone-event (`101`),
    /// comfort noise (`13`) when `comfort_noise_enabled = true`, and G.729
    /// (`18`) when the `g729` feature is enabled. Opus (`111`) and G.722 (`9`)
    /// remain post-beta or signaling-only experiments until media-core support
    /// is wired through and covered by interop/perf tests.
    ///
    /// Default: `vec![0, 8, 101]`.
    pub offered_codecs: Vec<u8>,

    /// G.729 Annex B VAD/DTX/CNG SDP preference for payload type 18.
    ///
    /// When PT 18 is present in [`Config::offered_codecs`] and the `g729`
    /// feature is enabled, outgoing offers carry `a=fmtp:18 annexb=yes` when
    /// this is `true` and `a=fmtp:18 annexb=no` when this is `false`. Answers
    /// disable Annex B if either side advertises `annexb=no`.
    ///
    /// Default: `true` (G.729A speech plus Annex B).
    pub g729_annex_b: bool,

    /// Capacity for the legacy incoming-call compatibility channel.
    ///
    /// Modern [`CallbackPeer`](crate::api::callback_peer::CallbackPeer) and
    /// [`StreamPeer`](crate::api::stream_peer::StreamPeer) consumers receive
    /// incoming calls through the app event publisher. The compatibility
    /// receiver exposed by [`UnifiedCoordinator::get_incoming_call`] is still
    /// kept for lower-level callers, so the buffer must be large enough for
    /// bursts without becoming a hidden backpressure point in the dialog event
    /// handler. Default: `1000`.
    pub incoming_call_channel_capacity: usize,

    /// Capacity for the internal state-machine event channel.
    ///
    /// State transitions publish lightweight internal events that the session
    /// event handler consumes and maps onto public API events where needed.
    /// This buffer must absorb short bursts so SIP request processing does not
    /// block behind event fan-out during load tests. Default: `1000`.
    pub state_event_channel_capacity: usize,

    /// Capacity for SIP transport event channels.
    ///
    /// This controls the per-transport receive queue (UDP/TCP/WS) and the
    /// combined transport-manager queue feeding the transaction layer. It is
    /// intentionally larger than the app-facing queues because one call setup
    /// produces multiple SIP messages and retransmission bursts can otherwise
    /// backpressure the UDP receive loop. Default: `10000`.
    pub sip_transport_channel_capacity: usize,

    /// Optional SIP transport-manager forwarding worker count.
    ///
    /// `None` preserves the single per-transport event bridge. Values above
    /// `1` enable keyed sharding between transport receive/parse and
    /// transaction-manager ingress.
    pub sip_transport_dispatch_workers: Option<usize>,

    /// Optional SIP transport-manager forwarding queue capacity.
    ///
    /// `None` uses [`Config::sip_transport_channel_capacity`]. When dispatch
    /// workers are enabled, this capacity is divided across workers.
    pub sip_transport_dispatch_queue_capacity: Option<usize>,

    /// Optional SIP UDP receive socket buffer size (`SO_RCVBUF`) in bytes.
    ///
    /// `None` preserves the OS default, which is appropriate for clients and
    /// small servers. High-CPS server profiles should set this alongside the
    /// transport channel capacity so kernel UDP bursts do not overflow before
    /// the async receive loop can drain them.
    pub sip_udp_recv_buffer_size: Option<usize>,

    /// Optional SIP UDP send socket buffer size (`SO_SNDBUF`) in bytes.
    ///
    /// `None` preserves the OS default. Server deployments with large reply
    /// bursts can set this to match the receive-side sizing policy.
    pub sip_udp_send_buffer_size: Option<usize>,

    /// Optional UDP parse worker count for the SIP UDP receive path.
    ///
    /// `None` keeps the transport default. High-CPS UDP servers can set this
    /// when parsing/dispatch work behind the socket receive loop needs more
    /// parallelism.
    pub sip_udp_parse_workers: Option<usize>,

    /// Optional per-worker UDP parse queue capacity.
    ///
    /// `None` uses the SIP transport channel capacity. When set, this bounds
    /// how many datagrams each UDP parse worker may buffer before overload is
    /// counted and dropped explicitly.
    pub sip_udp_parse_queue_capacity: Option<usize>,

    /// Optional UDP parse worker dispatch strategy.
    ///
    /// `None` preserves the transport default (`SourceHash`). High-CPS perf
    /// tests can opt into `RoundRobin` when the traffic generator sends all
    /// calls from a single source socket and source hashing cannot fan out.
    pub sip_udp_parse_dispatch: Option<rvoip_sip_transport::UdpParseDispatch>,

    /// Capacity for the transaction-manager event channel consumed by dialog
    /// core.
    ///
    /// A small transaction event queue can block transaction processing while
    /// dialog/session cleanup catches up. Default: `10000`.
    pub transaction_event_channel_capacity: usize,

    /// Optional transaction-manager ingress worker count.
    ///
    /// `None` preserves the single receive/handle loop used by clients and
    /// ordinary endpoints. High-CPS servers can set this above `1` to fan out
    /// transaction handling by a stable call/transaction key while preserving
    /// per-call request ordering.
    pub sip_transaction_dispatch_workers: Option<usize>,

    /// Optional transaction-manager ingress queue capacity.
    ///
    /// `None` uses [`Config::transaction_event_channel_capacity`]. When
    /// dispatch workers are enabled, this capacity is divided across workers.
    pub sip_transaction_dispatch_queue_capacity: Option<usize>,

    /// Optional per-transaction command channel capacity.
    ///
    /// Each SIP transaction owns a private command queue for timer and
    /// state-machine messages. This is not a global call burst queue; raising it
    /// increases per-transaction memory. `None` uses
    /// [`Config::DEFAULT_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY`]. High-CPS
    /// tuned profiles may raise it when measured timer/state command pressure
    /// proves the default is too small.
    pub sip_transaction_command_channel_capacity: Option<usize>,

    /// Optional priority-lane burst limit for transaction ingress workers.
    ///
    /// This applies only when [`Config::sip_transaction_dispatch_workers`] is
    /// greater than `1`. The transaction dispatcher lets ACK and BYE requests
    /// jump ahead of older normal-lane work on their assigned worker, then
    /// gives one ready normal item a turn after this many consecutive priority
    /// items. `None` uses the transaction-layer default (`64`). Lower this
    /// when INVITE/CANCEL/response work must make progress during teardown
    /// storms; raise it when ACK/BYE latency is the dominant failure mode and
    /// normal-lane delay is acceptable.
    pub sip_transaction_dispatch_priority_burst_max: Option<usize>,

    /// Optional cached INVITE `2xx` retransmission maintenance budget.
    ///
    /// The transaction manager keeps a short-lived cache of successful INVITE
    /// responses so duplicate INVITEs can be answered without rebuilding the
    /// SIP response. This limit controls how many cached `2xx` responses the
    /// proactive maintenance task may retransmit per 100 ms tick. `None` uses
    /// the transaction-layer default (`2048`). Lower this to pace retransmit
    /// storms when UDP send pressure starves teardown work; raise it when the
    /// host send path has headroom and UAC timeout/dead-call volume is driven
    /// by uncleared INVITE `2xx` loss bursts.
    pub sip_invite_2xx_retransmit_max_due_per_tick: Option<usize>,

    /// Optional rvoip-sip-dialog transaction-event dispatch worker count.
    ///
    /// `None` preserves the single dialog event processor. High-CPS servers can
    /// set this above `1` to fan out transaction events by stable call key while
    /// preserving per-call dialog ordering.
    pub sip_dialog_dispatch_workers: Option<usize>,

    /// Optional rvoip-sip-dialog transaction-event dispatch queue capacity.
    ///
    /// `None` uses the dialog max-dialog capacity hint. When dispatch workers
    /// are enabled, this capacity is divided across workers.
    pub sip_dialog_dispatch_queue_capacity: Option<usize>,

    /// Capacity for the infra-common global cross-crate event bus used inside
    /// this coordinator.
    ///
    /// ACK, BYE, media, and app-session events all cross this bus before they
    /// reach their local consumers. The default is intentionally modest so
    /// ordinary clients and PBX apps do not retain a large app-event ring.
    /// High-CPS server profiles should size it with the other signaling queues
    /// so the event bridge does not drop cleanup-driving events.
    /// Default: [`Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY`].
    pub global_event_channel_capacity: usize,

    /// Number of async workers used to publish app-level session events onto
    /// the global infra-common event bus.
    ///
    /// Internal per-call waits use the lifecycle index directly, but public
    /// event subscribers still receive events through the global bus. This
    /// worker pool avoids spawning one task per non-terminal event under
    /// server load. Default: logical CPU count capped at 16.
    pub session_event_dispatcher_workers: usize,

    /// Per-worker queue capacity for app-level session event publication.
    ///
    /// This bounded queue sits in front of the global event coordinator for
    /// every app event. Terminal publication awaits both queue admission and
    /// the worker's delivery acknowledgement, so exact session cleanup begins
    /// only after the terminal event's ordered publication attempt completes.
    /// Default: [`Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY`].
    pub session_event_dispatcher_channel_capacity: usize,

    /// Expected server-side active call capacity for hot lookup indexes.
    ///
    /// `None` keeps client/small-endpoint behavior lazy and uses existing
    /// channel-derived defaults. Server/high-CPS profiles should set this to
    /// the expected active-call burst size so dialog, transaction, session,
    /// lifecycle, and media indexes can reserve capacity up front without
    /// tying that memory reservation to the larger event-queue capacities.
    pub server_call_capacity: Option<usize>,

    /// Maximum active, quarantined, and retired SIP lifecycle records retained
    /// by the session authority.
    ///
    /// This is distinct from [`Config::server_call_capacity`], which limits
    /// simultaneous active lifetimes. A short-call, high-CPS server can retire
    /// many identifiers during the 64-second SIP anti-reuse horizon even when
    /// its active concurrency remains modest. Set this to at least the active
    /// capacity plus the expected call-arrival rate multiplied by that horizon,
    /// with suitable burst headroom. It requires `server_call_capacity` and
    /// must be greater than or equal to it. `None` preserves the library's
    /// conservative default retained-capacity calculation.
    pub server_retained_lifecycle_capacity: Option<usize>,

    /// Server-side admission limit for concurrently retained SIP call sessions.
    ///
    /// This is an enforcement knob, not a preallocation hint. When set and the
    /// session store is already at or above this limit, new inbound INVITEs are
    /// rejected by the library with a SIP overload response instead of being
    /// accepted into unbounded work. Server performance recipes should size
    /// this high enough for the expected CPS multiplied by call lifetime.
    /// `None` preserves endpoint/client behavior with no library admission cap.
    pub server_call_admission_limit: Option<usize>,

    /// Soft threshold for server-side admission pacing.
    ///
    /// When set and the active retained session count is at or above this
    /// value but below [`Config::server_call_admission_limit`], inbound INVITE
    /// processing is delayed by
    /// [`Config::server_call_admission_pacing_delay_ms`]. This gives active
    /// calls a chance to drain before the hard overload policy starts
    /// rejecting. `None` disables soft pacing.
    pub server_call_admission_soft_limit: Option<usize>,

    /// Delay applied when the server admission soft threshold is reached.
    ///
    /// Applies only when [`Config::server_call_admission_soft_limit`] is set.
    /// Values below `1` are rejected when set.
    pub server_call_admission_pacing_delay_ms: Option<u64>,

    /// `Retry-After` value in seconds for Config-owned server overload
    /// rejections.
    ///
    /// Applies only when [`Config::server_call_admission_limit`] is set and an
    /// inbound INVITE arrives while the server is at capacity. `Some(n)` adds
    /// `Retry-After: n` to the default `503 Service Unavailable` response;
    /// `None` omits the header. Default is `Some(1)`.
    pub server_overload_retry_after_secs: Option<u32>,

    /// Enable SIP UDP transport and duplicate-recovery diagnostics.
    ///
    /// This is a Config-owned replacement for benchmark-only diagnostic env
    /// toggles. It enables UDP receive/send counters and SIP duplicate
    /// INVITE/BYE cache counters for this process.
    pub sip_udp_diagnostics: bool,

    /// Enable high-cardinality transaction timing diagnostics.
    ///
    /// This records per-message transaction dispatch, handler, transaction
    /// creation, existing-transaction dispatch, and event-send histograms. It
    /// is intentionally separate from [`Config::sip_udp_diagnostics`] because
    /// it adds hot-path timestamp and atomic work under high CPS.
    pub sip_transaction_timing_diagnostics: bool,

    /// Enable high-cardinality dialog timing diagnostics.
    ///
    /// This records transaction-event-to-dialog queueing, dialog handler,
    /// dialog lookup, and dialog-to-session publish histograms. It is separate
    /// from transaction timing so 20k CPS tests can isolate the current hot
    /// layer.
    pub sip_dialog_timing_diagnostics: bool,

    /// Enable media setup/teardown timing diagnostics.
    ///
    /// This records media start/stop, RTP port allocation, RTP session
    /// creation, event subscription, and handler-spawn timing.
    pub media_setup_diagnostics: bool,

    /// Enable cleanup-stage timing diagnostics.
    ///
    /// This records cleanup and high-rate call-progress subpath counters used
    /// by the perf listener and high-CPS investigations.
    pub cleanup_diagnostics: bool,

    /// Enable per-operation cleanup diagnostic event logs.
    ///
    /// This is intentionally separate from [`Config::cleanup_diagnostics`]
    /// because it emits one log line per measured operation and is much more
    /// expensive under load.
    pub cleanup_diagnostic_events: bool,

    /// Maximum allowed RSS growth in MB/hour for perf soak release gates.
    ///
    /// This is compiled only with the `perf-tests` feature because it is a
    /// benchmark/release-gate control, not a production runtime memory limit.
    /// `None` uses [`Config::DEFAULT_PERF_MAX_RSS_GROWTH_MB_PER_HR`].
    #[cfg(feature = "perf-tests")]
    pub perf_max_rss_growth_mb_per_hr: Option<f64>,

    /// Enable SRTP negotiation diagnostic log lines.
    pub srtp_diagnostics: bool,

    /// Enable RTP packet diagnostic log lines.
    pub rtp_diagnostics: bool,

    /// Enable SDP media diagnostic log lines.
    pub media_sdp_diagnostics: bool,

    /// SIP_API_DESIGN_2 §7.4 — application-supplied headers stamped
    /// on every outbound message the state machine emits
    /// **automatically** (session-timer auto-BYE, dialog-terminated-
    /// during-INVITE auto-CANCEL, REFER-completion auto-NOTIFY).
    ///
    /// Stack-managed names (`Call-ID`, `CSeq`, `Via`, `Max-Forwards`,
    /// `Content-Length`, `Record-Route`) are rejected at
    /// [`Config::validate`] time. Method-shaped names that have a
    /// dedicated builder setter (e.g. `Authorization`) are accepted
    /// here — auto-emit messages have no per-call builder to route
    /// through.
    ///
    /// Applies to auto-emitted messages only; application-initiated
    /// builders inherit `Config` defaults through the §6.1 merge
    /// table, not through this field. The §7.4 precedence rule is:
    /// the state machine's auto-emit handler checks
    /// `pending_<method>_options` stash first; if populated those
    /// win and `auto_emit_extra_headers` is **not** appended.
    pub auto_emit_extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Config")
            .field("sip_port", &self.sip_port)
            .field("media_port_start", &self.media_port_start)
            .field("media_port_end", &self.media_port_end)
            .field("media_port_capacity", &self.media_port_capacity)
            .field("use_100rel", &self.use_100rel)
            .field("auto_180_ringing", &self.auto_180_ringing)
            .field("auto_100_trying", &self.auto_100_trying)
            .field(
                "fast_auto_accept_incoming_calls",
                &self.fast_auto_accept_incoming_calls,
            )
            .field(
                "setup_teardown_timeout_secs",
                &self.setup_teardown_timeout_secs,
            )
            .field("session_timer_secs", &self.session_timer_secs)
            .field("session_timer_min_se", &self.session_timer_min_se)
            .field("credentials_configured", &self.credentials.is_some())
            .field("auth_configured", &self.auth.is_some())
            .field("pai_configured", &self.pai_uri.is_some())
            .field("sip_trace_enabled", &self.sip_trace.enabled)
            .field("sip_trace_capacity", &self.sip_trace.capacity)
            .field(
                "sip_trace_sensitive_redaction",
                &self.sip_trace.redact_sensitive_headers,
            )
            .field("sip_trace_include_body", &self.sip_trace.include_body)
            .field(
                "trace_redaction_configured",
                &self.trace_redaction.is_some(),
            )
            .field(
                "outbound_proxy_configured",
                &self.outbound_proxy_uri.is_some(),
            )
            .field("sip_outbound_enabled", &self.sip_outbound_enabled)
            .field("sip_instance_configured", &self.sip_instance.is_some())
            .field("sip_tls_mode", &self.sip_tls_mode)
            .field("sip_contact_mode", &self.sip_contact_mode)
            .field("tls_bind_configured", &self.tls_bind_addr.is_some())
            .field(
                "tls_advertised_configured",
                &self.tls_advertised_addr.is_some(),
            )
            .field("contact_uri_configured", &self.contact_uri.is_some())
            .field("tls_cert_configured", &self.tls_cert_path.is_some())
            .field("tls_key_configured", &self.tls_key_path.is_some())
            .field(
                "tls_client_cert_configured",
                &self.tls_client_cert_path.is_some(),
            )
            .field(
                "tls_client_key_configured",
                &self.tls_client_key_path.is_some(),
            )
            .field("tls_extra_ca_configured", &self.tls_extra_ca_path.is_some())
            .field(
                "tls_server_client_auth_mode",
                &self.tls_server_client_auth.mode,
            )
            .field("offer_srtp", &self.offer_srtp)
            .field("srtp_required", &self.srtp_required)
            .field("srtp_suite_count", &self.srtp_offered_suites.len())
            .field(
                "media_public_address_configured",
                &self.media_public_addr.is_some(),
            )
            .field("media_mode", &self.media_mode)
            .field("media_session_capacity", &self.media_session_capacity)
            .field("stun_configured", &self.stun_server.is_some())
            .field("offered_codec_count", &self.offered_codecs.len())
            .field(
                "incoming_call_channel_capacity",
                &self.incoming_call_channel_capacity,
            )
            .field(
                "state_event_channel_capacity",
                &self.state_event_channel_capacity,
            )
            .field(
                "sip_transport_channel_capacity",
                &self.sip_transport_channel_capacity,
            )
            .field(
                "transaction_event_channel_capacity",
                &self.transaction_event_channel_capacity,
            )
            .field(
                "global_event_channel_capacity",
                &self.global_event_channel_capacity,
            )
            .field("server_call_capacity", &self.server_call_capacity)
            .field(
                "server_retained_lifecycle_capacity",
                &self.server_retained_lifecycle_capacity,
            )
            .field(
                "server_call_admission_limit",
                &self.server_call_admission_limit,
            )
            .field("sip_udp_diagnostics", &self.sip_udp_diagnostics)
            .field(
                "sip_transaction_timing_diagnostics",
                &self.sip_transaction_timing_diagnostics,
            )
            .field(
                "sip_dialog_timing_diagnostics",
                &self.sip_dialog_timing_diagnostics,
            )
            .field("media_setup_diagnostics", &self.media_setup_diagnostics)
            .field("cleanup_diagnostics", &self.cleanup_diagnostics)
            .field(
                "auto_emit_extra_header_count",
                &self.auto_emit_extra_headers.len(),
            )
            .finish_non_exhaustive()
    }
}

impl Config {
    /// Default RTP media port range start.
    pub const DEFAULT_MEDIA_PORT_START: u16 = DEFAULT_RTP_PORT_RANGE_START;

    /// Default RTP media port range end.
    pub const DEFAULT_MEDIA_PORT_END: u16 = DEFAULT_RTP_PORT_RANGE_END;

    /// Default RSS growth threshold for perf soak release gates.
    pub const DEFAULT_PERF_MAX_RSS_GROWTH_MB_PER_HR: f64 = 10.0;

    /// Default app-facing event buffer capacity.
    ///
    /// This is intentionally smaller than the lower-level SIP transport and
    /// transaction defaults so ordinary client/server apps do not reserve a
    /// large broadcast ring unless they opt into a high-CPS profile.
    pub const DEFAULT_APP_EVENT_CHANNEL_CAPACITY: usize = 256;

    /// Default per-transaction command channel capacity.
    pub const DEFAULT_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY: usize =
        rvoip_sip_dialog::transaction::DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY;

    /// Default watchdog timeout for setup/teardown states that are waiting on
    /// dialog-core terminal events.
    pub const DEFAULT_SETUP_TEARDOWN_TIMEOUT_SECS: u64 = 120;

    /// Explicitly allow verbatim SIP trace headers and any included bodies for
    /// controlled development/operator diagnostics.
    ///
    /// This disables both the pluggable default policy and the built-in
    /// sensitive-header/body-key redaction. It can expose credentials, PII,
    /// application context, and SDP key material. Do not use it in production.
    pub fn trace_passthrough_for_development(mut self) -> Self {
        self.trace_redaction = Some(std::sync::Arc::new(
            crate::api::trace_redactor::PassthroughRedactor,
        ));
        self.sip_trace = self.sip_trace.verbatim_for_development();
        self
    }

    /// Create a config for local development/testing on 127.0.0.1.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let config = Config::local("alice", 5060);
    /// assert_eq!(config.local_uri, "sip:alice@127.0.0.1:5060");
    /// ```
    pub fn local(name: &str, port: u16) -> Self {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        Self {
            local_ip: ip,
            sip_port: port,
            media_port_start: Self::DEFAULT_MEDIA_PORT_START,
            media_port_end: Self::DEFAULT_MEDIA_PORT_END,
            media_port_capacity: None,
            bind_addr: SocketAddr::new(ip, port),
            sip_advertised_addr: None,
            state_table_path: None,
            local_uri: format!("sip:{}@{}:{}", name, ip, port),
            use_100rel: RelUsage::default(),
            auto_180_ringing: true,
            auto_100_trying: true,
            fast_auto_accept_incoming_calls: false,
            setup_teardown_timeout_secs: Self::DEFAULT_SETUP_TEARDOWN_TIMEOUT_SECS,
            active_call_no_media_timeout_secs: 0,
            active_call_media_idle_timeout_secs: 0,
            session_timer_secs: None,
            session_timer_min_se: 90,
            credentials: None,
            auth: None,
            pai_uri: None,
            sip_trace: crate::api::events::SipTraceConfig::default(),
            trace_redaction: Some(crate::api::trace_redactor::default_trace_redactor()),
            outbound_proxy_uri: None,
            sip_outbound_enabled: false,
            sip_instance: None,
            outbound_keepalive_interval_secs: 25,
            registration_auto_refresh: true,
            registration_refresh_jitter_percent: 5,
            unregister_on_shutdown_timeout_secs: 3,
            sip_tls_mode: SipTlsMode::Disabled,
            sip_contact_mode: SipContactMode::ReachableContact,
            tls_bind_addr: None,
            tls_advertised_addr: None,
            contact_uri: None,
            tls_cert_path: None,
            tls_key_path: None,
            tls_client_cert_path: None,
            tls_client_key_path: None,
            tls_extra_ca_path: None,
            tls_server_client_auth:
                rvoip_sip_transport::transport::tls::TlsServerClientAuthConfig::default(),
            #[cfg(feature = "dev-insecure-tls")]
            tls_insecure_skip_verify: false,
            offer_srtp: false,
            srtp_required: false,
            srtp_offered_suites: SrtpSuitePolicy::Default.suites(),
            media_public_addr: None,
            media_mode: MediaMode::Enabled,
            media_session_capacity: None,
            rtp_session_buffer_config: RtpSessionBufferConfig::default(),
            rtp_transport_buffer_config: RtpTransportBufferConfig::default(),
            media_session_controller_config: MediaSessionControllerConfig::default(),
            stun_server: None,
            comfort_noise_enabled: false,
            strict_codec_matching: true,
            offered_codecs: vec![0, 8, 101],
            g729_annex_b: true,
            incoming_call_channel_capacity: 1000,
            state_event_channel_capacity: 1000,
            sip_transport_channel_capacity: 10_000,
            sip_transport_dispatch_workers: None,
            sip_transport_dispatch_queue_capacity: None,
            sip_udp_recv_buffer_size: None,
            sip_udp_send_buffer_size: None,
            sip_udp_parse_workers: None,
            sip_udp_parse_queue_capacity: None,
            sip_udp_parse_dispatch: None,
            transaction_event_channel_capacity: 10_000,
            sip_transaction_dispatch_workers: None,
            sip_transaction_dispatch_queue_capacity: None,
            sip_transaction_command_channel_capacity: None,
            sip_transaction_dispatch_priority_burst_max: None,
            sip_invite_2xx_retransmit_max_due_per_tick: None,
            sip_dialog_dispatch_workers: None,
            sip_dialog_dispatch_queue_capacity: None,
            global_event_channel_capacity: Self::DEFAULT_APP_EVENT_CHANNEL_CAPACITY,
            session_event_dispatcher_workers: default_session_event_dispatcher_workers(),
            session_event_dispatcher_channel_capacity: Self::DEFAULT_APP_EVENT_CHANNEL_CAPACITY,
            server_call_capacity: None,
            server_retained_lifecycle_capacity: None,
            server_call_admission_limit: None,
            server_call_admission_soft_limit: None,
            server_call_admission_pacing_delay_ms: None,
            server_overload_retry_after_secs: Some(1),
            sip_udp_diagnostics: false,
            sip_transaction_timing_diagnostics: false,
            sip_dialog_timing_diagnostics: false,
            media_setup_diagnostics: false,
            cleanup_diagnostics: false,
            cleanup_diagnostic_events: false,
            #[cfg(feature = "perf-tests")]
            perf_max_rss_growth_mb_per_hr: None,
            srtp_diagnostics: false,
            rtp_diagnostics: false,
            media_sdp_diagnostics: false,
            auto_emit_extra_headers: Vec::new(),
        }
    }

    /// Create a config bound to a specific IP address (e.g. for LAN or production).
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let config = Config::on("alice", "192.168.1.50".parse().unwrap(), 5060);
    /// assert_eq!(config.local_uri, "sip:alice@192.168.1.50:5060");
    /// ```
    pub fn on(name: &str, ip: IpAddr, port: u16) -> Self {
        Self {
            local_ip: ip,
            sip_port: port,
            media_port_start: Self::DEFAULT_MEDIA_PORT_START,
            media_port_end: Self::DEFAULT_MEDIA_PORT_END,
            media_port_capacity: None,
            bind_addr: SocketAddr::new(ip, port),
            sip_advertised_addr: None,
            state_table_path: None,
            local_uri: format!("sip:{}@{}:{}", name, ip, port),
            use_100rel: RelUsage::default(),
            auto_180_ringing: true,
            auto_100_trying: true,
            fast_auto_accept_incoming_calls: false,
            setup_teardown_timeout_secs: Self::DEFAULT_SETUP_TEARDOWN_TIMEOUT_SECS,
            active_call_no_media_timeout_secs: 0,
            active_call_media_idle_timeout_secs: 0,
            session_timer_secs: None,
            session_timer_min_se: 90,
            credentials: None,
            auth: None,
            pai_uri: None,
            sip_trace: crate::api::events::SipTraceConfig::default(),
            trace_redaction: Some(crate::api::trace_redactor::default_trace_redactor()),
            outbound_proxy_uri: None,
            sip_outbound_enabled: false,
            sip_instance: None,
            outbound_keepalive_interval_secs: 25,
            registration_auto_refresh: true,
            registration_refresh_jitter_percent: 5,
            unregister_on_shutdown_timeout_secs: 3,
            sip_tls_mode: SipTlsMode::Disabled,
            sip_contact_mode: SipContactMode::ReachableContact,
            tls_bind_addr: None,
            tls_advertised_addr: None,
            contact_uri: None,
            tls_cert_path: None,
            tls_key_path: None,
            tls_client_cert_path: None,
            tls_client_key_path: None,
            tls_extra_ca_path: None,
            tls_server_client_auth:
                rvoip_sip_transport::transport::tls::TlsServerClientAuthConfig::default(),
            #[cfg(feature = "dev-insecure-tls")]
            tls_insecure_skip_verify: false,
            offer_srtp: false,
            srtp_required: false,
            srtp_offered_suites: SrtpSuitePolicy::Default.suites(),
            media_public_addr: None,
            media_mode: MediaMode::Enabled,
            media_session_capacity: None,
            rtp_session_buffer_config: RtpSessionBufferConfig::default(),
            rtp_transport_buffer_config: RtpTransportBufferConfig::default(),
            media_session_controller_config: MediaSessionControllerConfig::default(),
            stun_server: None,
            comfort_noise_enabled: false,
            strict_codec_matching: true,
            offered_codecs: vec![0, 8, 101],
            g729_annex_b: true,
            incoming_call_channel_capacity: 1000,
            state_event_channel_capacity: 1000,
            sip_transport_channel_capacity: 10_000,
            sip_transport_dispatch_workers: None,
            sip_transport_dispatch_queue_capacity: None,
            sip_udp_recv_buffer_size: None,
            sip_udp_send_buffer_size: None,
            sip_udp_parse_workers: None,
            sip_udp_parse_queue_capacity: None,
            sip_udp_parse_dispatch: None,
            transaction_event_channel_capacity: 10_000,
            sip_transaction_dispatch_workers: None,
            sip_transaction_dispatch_queue_capacity: None,
            sip_transaction_command_channel_capacity: None,
            sip_transaction_dispatch_priority_burst_max: None,
            sip_invite_2xx_retransmit_max_due_per_tick: None,
            sip_dialog_dispatch_workers: None,
            sip_dialog_dispatch_queue_capacity: None,
            global_event_channel_capacity: Self::DEFAULT_APP_EVENT_CHANNEL_CAPACITY,
            session_event_dispatcher_workers: default_session_event_dispatcher_workers(),
            session_event_dispatcher_channel_capacity: Self::DEFAULT_APP_EVENT_CHANNEL_CAPACITY,
            server_call_capacity: None,
            server_retained_lifecycle_capacity: None,
            server_call_admission_limit: None,
            server_call_admission_soft_limit: None,
            server_call_admission_pacing_delay_ms: None,
            server_overload_retry_after_secs: Some(1),
            sip_udp_diagnostics: false,
            sip_transaction_timing_diagnostics: false,
            sip_dialog_timing_diagnostics: false,
            media_setup_diagnostics: false,
            cleanup_diagnostics: false,
            cleanup_diagnostic_events: false,
            #[cfg(feature = "perf-tests")]
            perf_max_rss_growth_mb_per_hr: None,
            srtp_diagnostics: false,
            rtp_diagnostics: false,
            media_sdp_diagnostics: false,
            auto_emit_extra_headers: Vec::new(),
        }
    }

    /// Deployment profile for local examples and integration tests.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let config = Config::local_lab("alice", 5060);
    /// assert_eq!(config.local_uri, "sip:alice@127.0.0.1:5060");
    /// ```
    pub fn local_lab(name: &str, port: u16) -> Self {
        Self::local(name, port)
    }

    /// Deployment profile for a directly reachable LAN PBX endpoint.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let bind = "0.0.0.0:5060".parse().unwrap();
    /// let advertised = "192.168.1.50:5060".parse().unwrap();
    /// let config = Config::lan_pbx("alice", bind, advertised);
    /// assert_eq!(config.sip_advertised_addr, Some(advertised));
    /// ```
    pub fn lan_pbx(name: &str, bind_addr: SocketAddr, advertised_addr: SocketAddr) -> Self {
        let mut config = Self::on(name, bind_addr.ip(), bind_addr.port());
        config.bind_addr = bind_addr;
        config.sip_advertised_addr = Some(advertised_addr);
        config.media_public_addr = Some(SocketAddr::new(advertised_addr.ip(), 0));
        config
    }

    /// Deployment profile for Asterisk TLS + SDES-SRTP with registered-flow
    /// reuse over the outbound registration connection.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SipTlsMode};
    /// let bind = "0.0.0.0:5061".parse().unwrap();
    /// let config = Config::asterisk_tls_registered_flow(
    ///     "alice",
    ///     bind,
    ///     "urn:uuid:00000000-0000-0000-0000-000000000001",
    /// );
    /// assert_eq!(config.sip_tls_mode, SipTlsMode::ClientOnly);
    /// assert!(config.srtp_required);
    /// ```
    pub fn asterisk_tls_registered_flow(
        name: &str,
        bind_addr: SocketAddr,
        sip_instance: impl Into<String>,
    ) -> Self {
        let mut config = Self::on(name, bind_addr.ip(), bind_addr.port())
            .tls_registered_flow_symmetric(sip_instance);
        config.bind_addr = bind_addr;
        config.offer_srtp = true;
        config.srtp_required = true;
        config
    }

    /// Deployment profile for FreeSWITCH/Sofia's internal LAN profile.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let bind = "192.168.1.50:5060".parse().unwrap();
    /// let config = Config::freeswitch_internal("alice", bind);
    /// assert!(config.strict_codec_matching);
    /// ```
    pub fn freeswitch_internal(name: &str, bind_addr: SocketAddr) -> Self {
        let mut config = Self::on(name, bind_addr.ip(), bind_addr.port());
        config.bind_addr = bind_addr;
        config.strict_codec_matching = true;
        config
    }

    /// Deployment profile for FreeSWITCH TLS + mandatory SDES-SRTP with a
    /// directly reachable TLS Contact.
    ///
    /// The profile enables SIP TLS listener mode, mandatory SRTP, strict codec
    /// matching, and the FreeSWITCH-compatible SDES suite policy. It does not
    /// pin a single crypto suite; SDP offer/answer decides the negotiated
    /// suite.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let udp_bind = "0.0.0.0:5060".parse().unwrap();
    /// let tls_bind = "0.0.0.0:5061".parse().unwrap();
    /// let config = Config::freeswitch_tls_srtp_reachable_contact(
    ///     "alice",
    ///     udp_bind,
    ///     tls_bind,
    ///     "cert.pem",
    ///     "key.pem",
    /// );
    /// assert!(config.offer_srtp);
    /// assert!(config.srtp_required);
    /// ```
    pub fn freeswitch_tls_srtp_reachable_contact(
        name: &str,
        bind_addr: SocketAddr,
        tls_bind_addr: SocketAddr,
        cert_path: impl Into<std::path::PathBuf>,
        key_path: impl Into<std::path::PathBuf>,
    ) -> Self {
        let mut config = Self::freeswitch_internal(name, bind_addr)
            .tls_reachable_contact(tls_bind_addr, cert_path, key_path)
            .with_srtp_suite_policy(SrtpSuitePolicy::FreeSwitchCompatible);
        config.offer_srtp = true;
        config.srtp_required = true;
        config
    }

    /// Deployment profile for carrier/SBC style outbound proxy operation.
    ///
    /// This is a conservative starting point: TLS client mode, registered-flow
    /// Contact behavior, mandatory SDES-SRTP, explicit public media address,
    /// and a preloaded outbound proxy route for INVITEs. REGISTER proxy,
    /// Service-Route/Path, SRV/NAPTR, and ICE remain separate hardening work.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SipContactMode};
    /// let bind = "0.0.0.0:5061".parse().unwrap();
    /// let public = "203.0.113.10:5061".parse().unwrap();
    /// let config = Config::carrier_sbc(
    ///     "alice",
    ///     bind,
    ///     public,
    ///     "sips:sbc.example.com:5061;lr",
    ///     "urn:uuid:00000000-0000-0000-0000-000000000001",
    /// );
    /// assert_eq!(config.sip_contact_mode, SipContactMode::RegisteredFlowRfc5626);
    /// assert!(config.srtp_required);
    /// ```
    pub fn carrier_sbc(
        name: &str,
        bind_addr: SocketAddr,
        public_addr: SocketAddr,
        outbound_proxy_uri: impl Into<String>,
        sip_instance: impl Into<String>,
    ) -> Self {
        let mut config = Self::on(name, bind_addr.ip(), bind_addr.port())
            .tls_registered_flow_rfc5626(sip_instance);
        config.bind_addr = bind_addr;
        config.sip_advertised_addr = Some(public_addr);
        config.tls_advertised_addr = Some(public_addr);
        config.media_public_addr = Some(SocketAddr::new(public_addr.ip(), 0));
        config.outbound_proxy_uri = Some(outbound_proxy_uri.into());
        config.offer_srtp = true;
        config.srtp_required = true;
        config
    }

    /// Placeholder deployment profile for a SIP proxy plus RTPengine lab.
    ///
    /// The signaling side preloads the outbound proxy route; media relay
    /// integration remains explicit because RTPengine control belongs above
    /// rvoip-sip.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let bind = "0.0.0.0:5060".parse().unwrap();
    /// let advertised = "192.168.1.50:5060".parse().unwrap();
    /// let config = Config::proxy_rtpengine(
    ///     "alice",
    ///     bind,
    ///     advertised,
    ///     "sip:proxy.example.com;lr",
    /// );
    /// assert_eq!(config.outbound_proxy_uri.as_deref(), Some("sip:proxy.example.com;lr"));
    /// ```
    pub fn proxy_rtpengine(
        name: &str,
        bind_addr: SocketAddr,
        advertised_addr: SocketAddr,
        outbound_proxy_uri: impl Into<String>,
    ) -> Self {
        let mut config = Self::lan_pbx(name, bind_addr, advertised_addr);
        config.outbound_proxy_uri = Some(outbound_proxy_uri.into());
        config
    }

    /// Replace the SDES-SRTP offer suite list with a named policy.
    ///
    /// This only changes the advertised suite order/list. Callers still choose
    /// whether SRTP is offered or mandatory with [`Config::offer_srtp`] and
    /// [`Config::srtp_required`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SrtpSuitePolicy};
    /// let config = Config::local("alice", 5060)
    ///     .with_srtp_suite_policy(SrtpSuitePolicy::FreeSwitchCompatible);
    /// assert_eq!(config.srtp_offered_suites.len(), 4);
    /// ```
    pub fn with_srtp_suite_policy(mut self, policy: SrtpSuitePolicy) -> Self {
        self.srtp_offered_suites = policy.suites();
        self
    }

    /// Set the G.729 Annex B SDP preference used when PT 18 is advertised.
    ///
    /// `true` emits `a=fmtp:18 annexb=yes` for G.729A plus Annex B
    /// VAD/DTX/CNG. `false` emits `a=fmtp:18 annexb=no` for G.729A speech
    /// only.
    pub fn with_g729_annex_b(mut self, enabled: bool) -> Self {
        self.g729_annex_b = enabled;
        self
    }

    /// Set the legacy incoming-call compatibility channel capacity.
    ///
    /// The default is `1000`, which is enough for normal bursty call-arrival
    /// workloads while still bounding memory. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_incoming_call_channel_capacity(mut self, capacity: usize) -> Self {
        self.incoming_call_channel_capacity = capacity;
        self
    }

    /// Set SIP signaling channel capacities from one expected-concurrency knob.
    ///
    /// `capacity` is the expected number of concurrent or burst-arriving calls.
    /// Per-call queues use that value directly; lower-level transport and
    /// transaction event queues use `capacity * 10` because a single call
    /// generates multiple SIP messages and transaction lifecycle events.
    /// Values below `1` are rejected by [`Config::validate`].
    pub fn with_channel_capacity(mut self, capacity: usize) -> Self {
        let event_capacity = capacity.saturating_mul(10);
        self.incoming_call_channel_capacity = capacity;
        self.state_event_channel_capacity = capacity;
        self.sip_transport_channel_capacity = event_capacity;
        self.transaction_event_channel_capacity = event_capacity;
        self.global_event_channel_capacity = event_capacity;
        self.session_event_dispatcher_channel_capacity = event_capacity;
        self
    }

    /// Set a server-side active-call capacity profile.
    ///
    /// This reserves hot lookup indexes for `capacity` active or
    /// burst-arriving calls without changing event queue sizes. Clients should
    /// usually leave this unset and use the defaults.
    pub fn with_server_capacity(mut self, capacity: usize) -> Self {
        self.server_call_capacity = Some(capacity);
        self
    }

    /// Set the bound for active and retained SIP lifecycle records.
    ///
    /// Configure [`Config::server_call_capacity`] as well. The retained bound
    /// must cover the active capacity and enough completed calls for the
    /// lifecycle anti-reuse horizon.
    pub fn with_server_retained_lifecycle_capacity(mut self, capacity: usize) -> Self {
        self.server_retained_lifecycle_capacity = Some(capacity);
        self
    }

    /// Set the server-side active-call admission limit.
    ///
    /// Unlike [`Config::with_server_capacity`], this is enforced at runtime for
    /// inbound INVITEs. Once the session store reaches `limit`, additional
    /// inbound calls are rejected with SIP `503 Service Unavailable` and the
    /// configured `Retry-After` header.
    pub fn with_server_call_admission_limit(mut self, limit: usize) -> Self {
        self.server_call_admission_limit = Some(limit);
        self
    }

    /// Set the soft threshold where server-side inbound admission starts
    /// pacing instead of immediately admitting new calls.
    pub fn with_server_call_admission_soft_limit(mut self, limit: usize) -> Self {
        self.server_call_admission_soft_limit = Some(limit);
        self
    }

    /// Set the delay used while server-side inbound admission is above the
    /// soft threshold but below the hard limit.
    pub fn with_server_call_admission_pacing_delay_ms(mut self, delay_ms: u64) -> Self {
        self.server_call_admission_pacing_delay_ms = Some(delay_ms);
        self
    }

    /// Remove the server-side active-call admission limit.
    pub fn without_server_call_admission_limit(mut self) -> Self {
        self.server_call_admission_limit = None;
        self
    }

    /// Set the `Retry-After` value used for server overload rejections.
    pub fn with_server_overload_retry_after_secs(mut self, seconds: u32) -> Self {
        self.server_overload_retry_after_secs = Some(seconds);
        self
    }

    /// Omit `Retry-After` from server overload rejections.
    pub fn without_server_overload_retry_after(mut self) -> Self {
        self.server_overload_retry_after_secs = None;
        self
    }

    /// Apply a high-CPS UDP auto-answer profile.
    ///
    /// This keeps media enabled, suppresses automatic provisional responses,
    /// sizes SIP event queues from `capacity`, and configures the UDP receive
    /// path for a single fast parse worker with a queue sized to the same
    /// burst capacity. It disables automatic `100 Trying` because fixed
    /// immediate-answer services should send the final response before Timer
    /// 100 would fire, and avoiding the timer task/message reduces hot-path
    /// work. It does not enable the fused fast-auto-accept path yet; that
    /// remains an explicit opt-in until the 8000 CPS cleanup/retransmit target
    /// is stable. It raises
    /// [`Config::sip_transaction_command_channel_capacity`] to
    /// `(capacity / 8).clamp(128, 1000)` unless an earlier setter already set
    /// an explicit value. It does not enlarge socket buffers and does not set
    /// [`Config::server_call_capacity`]. It also leaves
    /// [`Config::sip_transaction_dispatch_priority_burst_max`] and
    /// [`Config::sip_invite_2xx_retransmit_max_due_per_tick`] unset so load
    /// tests can tune dispatch fairness and retransmit pacing explicitly.
    pub fn with_high_cps_udp_auto_answer(mut self, capacity: usize) -> Self {
        self = self.with_channel_capacity(capacity);
        self.auto_180_ringing = false;
        self.auto_100_trying = false;
        self.sip_udp_parse_workers = Some(1);
        self.sip_udp_parse_queue_capacity = Some(capacity);
        self.sip_transaction_command_channel_capacity
            .get_or_insert_with(|| (capacity / 8).clamp(128, 1000));
        self.media_mode = MediaMode::Enabled;
        self
    }

    /// Apply a YAML-backed performance recipe to this config.
    ///
    /// When [`crate::PerformanceConfig::recipe_path`] is set, that YAML file
    /// is loaded. Otherwise the bundled default recipe book is used.
    pub fn try_with_performance_config(
        self,
        performance: crate::api::performance::PerformanceConfig,
    ) -> Result<Self> {
        let book = if let Some(path) = &performance.recipe_path {
            crate::api::performance::PerformanceRecipeBook::from_path(path)?
        } else {
            crate::api::performance::PerformanceRecipeBook::bundled()?
        };
        book.apply(self, &performance)
    }

    /// Apply the bundled PBX media server performance recipe.
    ///
    /// This convenience helper applies the bundled
    /// `pbx-media-server` YAML recipe. Use
    /// [`Config::try_with_performance_config`] with a custom
    /// [`crate::PerformanceConfig::recipe_path`] when deployments need a
    /// modified recipe book.
    pub fn with_pbx_media_server_performance(self, capacity: usize) -> Self {
        self.try_with_performance_config(
            crate::api::performance::PerformanceConfig::pbx_media_server(capacity),
        )
        .expect("bundled pbx-media-server performance recipe is valid")
    }

    /// Apply the bundled signaling-only high-performance server recipe.
    ///
    /// This convenience helper applies the bundled
    /// `signaling-only-server-high-performance` YAML recipe. Use
    /// [`Config::try_with_performance_config`] with a custom
    /// [`crate::PerformanceConfig::recipe_path`] when deployments need a
    /// modified recipe book.
    pub fn with_signaling_only_server_high_performance(
        self,
        capacity: usize,
        sdp_rtp_port: u16,
    ) -> Self {
        self.try_with_performance_config(
            crate::api::performance::PerformanceConfig::signaling_only_server_high_performance(
                capacity,
            )
            .with_signaling_only_rtp_port(sdp_rtp_port),
        )
        .expect("bundled signaling-only-server-high-performance performance recipe is valid")
    }

    fn dialog_index_capacity_hint(&self) -> usize {
        self.server_call_capacity
            .unwrap_or(self.transaction_event_channel_capacity)
            .max(1)
    }

    fn transaction_index_capacity_hint(&self) -> usize {
        self.server_call_capacity
            // INVITE server transactions, BYE server transactions, ACK
            // indexes, and retransmission caches can remain live beyond the
            // active-call count. Size for the short transaction-retention
            // window, not just simultaneous dialogs.
            .map(|capacity| capacity.saturating_mul(16))
            .unwrap_or(self.transaction_event_channel_capacity)
            .max(1)
    }

    /// Set the RTP media port range.
    ///
    /// The default range is [`Config::DEFAULT_MEDIA_PORT_START`] through
    /// [`Config::DEFAULT_MEDIA_PORT_END`]. Values are checked by
    /// [`Config::validate`].
    pub fn with_media_ports(mut self, start: u16, end: u16) -> Self {
        self.media_port_start = start;
        self.media_port_end = end;
        self.media_port_capacity = None;
        self
    }

    /// Set the RTP media port range by start port and requested capacity.
    ///
    /// Validation rejects capacity `0`, start ports below [`MIN_PORT`], and
    /// requested capacities that do not fit in the `u16` port space.
    pub fn with_media_port_capacity(mut self, start: u16, capacity: usize) -> Self {
        self.media_port_start = start;
        self.media_port_end = capacity
            .checked_sub(1)
            .and_then(|offset| (start as usize).checked_add(offset))
            .and_then(|end| u16::try_from(end).ok())
            .unwrap_or(u16::MAX);
        self.media_port_capacity = Some(capacity);
        self
    }

    /// Advertise a concrete peer-facing SIP address while retaining the configured local
    /// bind address. Useful for containers, 1:1 NAT, and host networking.
    pub fn with_sip_advertised_addr(mut self, address: SocketAddr) -> Self {
        self.sip_advertised_addr = Some(address);
        self
    }

    /// Return to bind-derived SIP Via/Contact generation.
    pub fn without_sip_advertised_addr(mut self) -> Self {
        self.sip_advertised_addr = None;
        self
    }

    /// Advertise a routable RTP address in SDP. Port `0` means retain each
    /// session's allocated local RTP port while replacing only the IP.
    pub fn with_media_public_addr(mut self, address: SocketAddr) -> Self {
        self.media_public_addr = Some(address);
        self
    }

    /// Return to local-address or STUN-derived RTP advertisement.
    pub fn without_media_public_addr(mut self) -> Self {
        self.media_public_addr = None;
        self
    }

    /// Enable or disable automatic `180 Ringing` on inbound INVITEs.
    ///
    /// `true` is the PBX-friendly default. `false` is useful for IVR,
    /// call-center, and benchmark listeners that answer immediately with a
    /// final response.
    pub fn with_auto_180_ringing(mut self, enabled: bool) -> Self {
        self.auto_180_ringing = enabled;
        self
    }

    /// Enable or disable the automatic RFC 3261 `100 Trying` timer.
    ///
    /// The default is `true`. High-CPS immediate-answer services can set this
    /// to `false` to avoid spawning a timer task for every INVITE when a final
    /// response is expected well before Timer 100 would fire.
    pub fn with_auto_100_trying(mut self, enabled: bool) -> Self {
        self.auto_100_trying = enabled;
        self
    }

    /// Enable or disable immediate session-path accept for inbound INVITEs.
    ///
    /// This is intentionally separate from [`Config::auto_180_ringing`]:
    /// disabling 180 only removes the provisional response, while enabling
    /// this option sends the final answer before app callbacks run.
    pub fn with_fast_auto_accept_incoming_calls(mut self, enabled: bool) -> Self {
        self.fast_auto_accept_incoming_calls = enabled;
        self
    }

    /// Set the watchdog timeout for setup teardown states.
    ///
    /// The watchdog only fires if a session is still in the same setup state
    /// after the timeout. It then drives the existing state-table
    /// `DialogTimeout` transition so cleanup and terminal event publication
    /// use the normal path. Use `0` to disable the watchdog.
    pub fn with_setup_teardown_timeout_secs(mut self, seconds: u64) -> Self {
        self.setup_teardown_timeout_secs = seconds;
        self
    }

    /// Set the watchdog timeout for answered inbound calls with no RTP.
    ///
    /// The watchdog is disabled when set to `0`. When enabled, it only arms
    /// for UAS calls that have reached `Active` with a media session and
    /// releases the call if the media session's RTP receive counter has not
    /// advanced by the timeout.
    pub fn with_active_call_no_media_timeout_secs(mut self, seconds: u64) -> Self {
        self.active_call_no_media_timeout_secs = seconds;
        self
    }

    /// Set the watchdog timeout for answered inbound calls whose RTP stops.
    ///
    /// The watchdog is disabled when set to `0`. When enabled, it only arms
    /// for UAS calls that have reached `Active` with a media session. The
    /// watchdog disarms while RTP packet counts advance and releases the call
    /// if the packet count stops advancing for the configured interval.
    pub fn with_active_call_media_idle_timeout_secs(mut self, seconds: u64) -> Self {
        self.active_call_media_idle_timeout_secs = seconds;
        self
    }

    /// Set media allocation behavior.
    pub fn with_media_mode(mut self, mode: MediaMode) -> Self {
        self.media_mode = mode;
        self
    }

    /// Set the media-core session and RTP allocator capacity hint.
    pub fn with_media_session_capacity(mut self, capacity: usize) -> Self {
        self.media_session_capacity = Some(capacity);
        self
    }

    /// Set RTP session queue sizing for SIP media calls.
    pub fn with_rtp_session_buffer_config(mut self, config: RtpSessionBufferConfig) -> Self {
        self.rtp_session_buffer_config = config;
        self
    }

    /// Set RTP transport event and receive buffer sizing for SIP media calls.
    pub fn with_rtp_transport_buffer_config(mut self, config: RtpTransportBufferConfig) -> Self {
        self.rtp_transport_buffer_config = config;
        self
    }

    /// Set media-core controller pool and capacity tuning for SIP media calls.
    pub fn with_media_session_controller_config(
        mut self,
        config: MediaSessionControllerConfig,
    ) -> Self {
        self.rtp_session_buffer_config = config.rtp_session_buffer_config;
        self.rtp_transport_buffer_config = config.rtp_transport_buffer_config;
        self.media_session_controller_config = config;
        self
    }

    /// Enable or disable real media-core RTP allocation.
    ///
    /// Disabling media switches to [`MediaMode::SignalingOnly`] with SDP port
    /// `9`, the discard port convention used for signaling-only tests.
    pub fn with_media_enabled(mut self, enabled: bool) -> Self {
        self.media_mode = if enabled {
            MediaMode::Enabled
        } else {
            MediaMode::SignalingOnly { sdp_rtp_port: 9 }
        };
        self
    }

    /// Skip media-core RTP allocation while still generating SDP.
    pub fn with_signaling_only_media(mut self, sdp_rtp_port: u16) -> Self {
        self.media_mode = MediaMode::SignalingOnly { sdp_rtp_port };
        self
    }

    /// Set the internal state-machine event channel capacity.
    ///
    /// The default is `1000`. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_state_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.state_event_channel_capacity = capacity;
        self
    }

    /// Set the SIP transport event channel capacity.
    ///
    /// The default is `10000`. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_sip_transport_channel_capacity(mut self, capacity: usize) -> Self {
        self.sip_transport_channel_capacity = capacity;
        self
    }

    /// Set the SIP transport-manager forwarding worker count.
    ///
    /// Values above `1` enable keyed sharding between transport receive/parse
    /// and transaction-manager ingress. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_sip_transport_dispatch_workers(mut self, workers: usize) -> Self {
        self.sip_transport_dispatch_workers = Some(workers);
        self
    }

    /// Set the SIP transport-manager forwarding queue capacity.
    ///
    /// `None` uses [`Config::sip_transport_channel_capacity`]. Values below
    /// `1` are rejected by [`Config::validate`].
    pub fn with_sip_transport_dispatch_queue_capacity(mut self, capacity: usize) -> Self {
        self.sip_transport_dispatch_queue_capacity = Some(capacity);
        self
    }

    /// Set SIP transport-manager forwarding worker and queue overrides
    /// together.
    pub fn with_sip_transport_dispatch_config(
        mut self,
        workers: Option<usize>,
        queue_capacity: Option<usize>,
    ) -> Self {
        self.sip_transport_dispatch_workers = workers;
        self.sip_transport_dispatch_queue_capacity = queue_capacity;
        self
    }

    /// Set SIP UDP socket receive/send buffer sizes in bytes.
    ///
    /// Use this for high-CPS server profiles where the kernel UDP queue must
    /// absorb bursts while application queues drain. Pass `None` for either
    /// side to keep that side at the OS default.
    pub fn with_sip_udp_socket_buffers(
        mut self,
        recv_buffer_size: Option<usize>,
        send_buffer_size: Option<usize>,
    ) -> Self {
        self.sip_udp_recv_buffer_size = recv_buffer_size;
        self.sip_udp_send_buffer_size = send_buffer_size;
        self
    }

    /// Set the SIP UDP receive socket buffer size (`SO_RCVBUF`) in bytes.
    pub fn with_sip_udp_recv_buffer_size(mut self, size: usize) -> Self {
        self.sip_udp_recv_buffer_size = Some(size);
        self
    }

    /// Set the SIP UDP send socket buffer size (`SO_SNDBUF`) in bytes.
    pub fn with_sip_udp_send_buffer_size(mut self, size: usize) -> Self {
        self.sip_udp_send_buffer_size = Some(size);
        self
    }

    /// Set the UDP parse worker count.
    pub fn with_sip_udp_parse_workers(mut self, workers: usize) -> Self {
        self.sip_udp_parse_workers = Some(workers);
        self
    }

    /// Set the per-worker UDP parse queue capacity.
    pub fn with_sip_udp_parse_queue_capacity(mut self, capacity: usize) -> Self {
        self.sip_udp_parse_queue_capacity = Some(capacity);
        self
    }

    /// Set the UDP parse worker dispatch strategy.
    pub fn with_sip_udp_parse_dispatch(
        mut self,
        dispatch: rvoip_sip_transport::UdpParseDispatch,
    ) -> Self {
        self.sip_udp_parse_dispatch = Some(dispatch);
        self
    }

    /// Set UDP parse worker and queue overrides together.
    pub fn with_sip_udp_parse_config(
        mut self,
        workers: Option<usize>,
        queue_capacity: Option<usize>,
    ) -> Self {
        self.sip_udp_parse_workers = workers;
        self.sip_udp_parse_queue_capacity = queue_capacity;
        self
    }

    /// Set the transaction-manager event channel capacity.
    ///
    /// The default is `10000`. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_transaction_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.transaction_event_channel_capacity = capacity;
        self
    }

    /// Set the transaction-manager ingress dispatch worker count.
    ///
    /// Values above `1` enable keyed sharding of incoming transport events.
    /// Values below `1` are rejected by [`Config::validate`].
    pub fn with_sip_transaction_dispatch_workers(mut self, workers: usize) -> Self {
        self.sip_transaction_dispatch_workers = Some(workers);
        self
    }

    /// Set the transaction-manager ingress dispatch queue capacity.
    ///
    /// `None` uses [`Config::transaction_event_channel_capacity`]. Values below
    /// `1` are rejected by [`Config::validate`].
    pub fn with_sip_transaction_dispatch_queue_capacity(mut self, capacity: usize) -> Self {
        self.sip_transaction_dispatch_queue_capacity = Some(capacity);
        self
    }

    /// Set the per-transaction command channel capacity.
    ///
    /// Values below `1` are rejected by [`Config::validate`]. This should be
    /// tuned only for measured high-CPS profiles because it is allocated per
    /// live transaction, not once per endpoint.
    pub fn with_sip_transaction_command_channel_capacity(mut self, capacity: usize) -> Self {
        self.sip_transaction_command_channel_capacity = Some(capacity);
        self
    }

    /// Set the transaction-manager ACK/BYE priority burst limit.
    ///
    /// This only affects multi-worker transaction dispatch. After this many
    /// consecutive priority-lane ACK/BYE events, a worker processes one ready
    /// normal-lane item before resuming priority work. Use lower values when
    /// INVITE/CANCEL/response work is being starved; use higher values when
    /// BYE/ACK latency is the bottleneck. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_sip_transaction_dispatch_priority_burst_max(mut self, max_burst: usize) -> Self {
        self.sip_transaction_dispatch_priority_burst_max = Some(max_burst);
        self
    }

    /// Set the cached INVITE `2xx` retransmission maintenance budget.
    ///
    /// The value is the maximum number of cached INVITE `2xx` responses the
    /// transaction manager may proactively resend per 100 ms tick. Lower values
    /// pace UDP send bursts; higher values clear retransmission backlog faster
    /// when the host send path has capacity. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_sip_invite_2xx_retransmit_max_due_per_tick(
        mut self,
        max_due_per_tick: usize,
    ) -> Self {
        self.sip_invite_2xx_retransmit_max_due_per_tick = Some(max_due_per_tick);
        self
    }

    /// Set transaction-manager ingress worker and queue overrides together.
    pub fn with_sip_transaction_dispatch_config(
        mut self,
        workers: Option<usize>,
        queue_capacity: Option<usize>,
    ) -> Self {
        self.sip_transaction_dispatch_workers = workers;
        self.sip_transaction_dispatch_queue_capacity = queue_capacity;
        self
    }

    /// Set the rvoip-sip-dialog transaction-event dispatch worker count.
    ///
    /// Values above `1` enable keyed sharding of transaction events before
    /// dialog protocol handling. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_sip_dialog_dispatch_workers(mut self, workers: usize) -> Self {
        self.sip_dialog_dispatch_workers = Some(workers);
        self
    }

    /// Set the rvoip-sip-dialog transaction-event dispatch queue capacity.
    ///
    /// `None` uses the dialog max-dialog capacity hint. Values below `1` are
    /// rejected by [`Config::validate`].
    pub fn with_sip_dialog_dispatch_queue_capacity(mut self, capacity: usize) -> Self {
        self.sip_dialog_dispatch_queue_capacity = Some(capacity);
        self
    }

    /// Set rvoip-sip-dialog transaction-event dispatch worker and queue overrides
    /// together.
    pub fn with_sip_dialog_dispatch_config(
        mut self,
        workers: Option<usize>,
        queue_capacity: Option<usize>,
    ) -> Self {
        self.sip_dialog_dispatch_workers = workers;
        self.sip_dialog_dispatch_queue_capacity = queue_capacity;
        self
    }

    /// Set the infra-common global event bus channel capacity.
    ///
    /// The default is `256` ([`Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY`]); the
    /// `EventCoordinatorConfig` native default of `10000` is overridden by the
    /// unified layer. Values below `1` are rejected by [`Config::validate`].
    pub fn with_global_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.global_event_channel_capacity = capacity;
        self
    }

    /// Set the app-facing event buffering capacity as one knob.
    ///
    /// This sets both [`Config::global_event_channel_capacity`] and
    /// [`Config::session_event_dispatcher_channel_capacity`]. Use the lower
    /// level setters when a deployment needs different capacities for the
    /// global cross-crate broadcast ring and the app-session publish queue.
    pub fn with_app_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.global_event_channel_capacity = capacity;
        self.session_event_dispatcher_channel_capacity = capacity;
        self
    }

    /// Set the app-session event dispatcher worker count.
    ///
    /// Values below `1` are rejected by [`Config::validate`].
    pub fn with_session_event_dispatcher_workers(mut self, workers: usize) -> Self {
        self.session_event_dispatcher_workers = workers;
        self
    }

    /// Set the per-worker app-session event dispatcher queue capacity.
    ///
    /// Values below `1` are rejected by [`Config::validate`].
    pub fn with_session_event_dispatcher_channel_capacity(mut self, capacity: usize) -> Self {
        self.session_event_dispatcher_channel_capacity = capacity;
        self
    }

    /// Enable or disable SIP UDP transport and duplicate-recovery diagnostics.
    pub fn with_sip_udp_diagnostics(mut self, enabled: bool) -> Self {
        self.sip_udp_diagnostics = enabled;
        self
    }

    /// Enable or disable high-cardinality transaction timing diagnostics.
    pub fn with_sip_transaction_timing_diagnostics(mut self, enabled: bool) -> Self {
        self.sip_transaction_timing_diagnostics = enabled;
        self
    }

    /// Enable or disable high-cardinality dialog timing diagnostics.
    pub fn with_sip_dialog_timing_diagnostics(mut self, enabled: bool) -> Self {
        self.sip_dialog_timing_diagnostics = enabled;
        self
    }

    /// Enable or disable media setup/teardown timing diagnostics.
    pub fn with_media_setup_diagnostics(mut self, enabled: bool) -> Self {
        self.media_setup_diagnostics = enabled;
        self
    }

    /// Enable or disable cleanup-stage timing diagnostics.
    pub fn with_cleanup_diagnostics(mut self, enabled: bool) -> Self {
        self.cleanup_diagnostics = enabled;
        self
    }

    /// Enable or disable per-operation cleanup diagnostic event logs.
    pub fn with_cleanup_diagnostic_events(mut self, enabled: bool) -> Self {
        self.cleanup_diagnostic_events = enabled;
        self
    }

    /// Set the RSS growth threshold used by perf soak release gates.
    ///
    /// Compiled only with `perf-tests`; ordinary application builds do not
    /// expose benchmark gate controls.
    #[cfg(feature = "perf-tests")]
    pub fn with_perf_max_rss_growth_mb_per_hr(mut self, limit: f64) -> Self {
        self.perf_max_rss_growth_mb_per_hr = Some(limit);
        self
    }

    /// Enable or disable SRTP negotiation diagnostic log lines.
    pub fn with_srtp_diagnostics(mut self, enabled: bool) -> Self {
        self.srtp_diagnostics = enabled;
        self
    }

    /// Enable or disable RTP packet diagnostic log lines.
    pub fn with_rtp_diagnostics(mut self, enabled: bool) -> Self {
        self.rtp_diagnostics = enabled;
        self
    }

    /// Enable or disable SDP media diagnostic log lines.
    pub fn with_media_sdp_diagnostics(mut self, enabled: bool) -> Self {
        self.media_sdp_diagnostics = enabled;
        self
    }

    /// Configure SIP TLS as a directly reachable Contact listener.
    ///
    /// The UA will both dial outbound TLS and listen on `tls_bind_addr` for
    /// inbound TLS requests sent to its advertised Contact.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SipContactMode, SipTlsMode};
    /// let tls_addr = "0.0.0.0:5061".parse().unwrap();
    /// let config = Config::local("alice", 5060)
    ///     .tls_reachable_contact(tls_addr, "cert.pem", "key.pem");
    /// assert_eq!(config.sip_tls_mode, SipTlsMode::ClientAndServer);
    /// assert_eq!(config.sip_contact_mode, SipContactMode::ReachableContact);
    /// ```
    pub fn tls_reachable_contact(
        mut self,
        tls_bind_addr: SocketAddr,
        cert_path: impl Into<std::path::PathBuf>,
        key_path: impl Into<std::path::PathBuf>,
    ) -> Self {
        self.sip_tls_mode = SipTlsMode::ClientAndServer;
        self.sip_contact_mode = SipContactMode::ReachableContact;
        self.tls_bind_addr = Some(tls_bind_addr);
        if self.tls_advertised_addr.is_none() && !tls_bind_addr.ip().is_unspecified() {
            self.tls_advertised_addr = Some(tls_bind_addr);
        }
        self.tls_cert_path = Some(cert_path.into());
        self.tls_key_path = Some(key_path.into());
        self
    }

    /// Configure how the inbound SIP TLS listener verifies client
    /// certificates.
    ///
    /// `Optional` and `Required` policies must name a PEM client-CA bundle;
    /// [`Config::validate`] rejects incomplete configurations. Use this with
    /// a tenant-bound [`crate::SipListenerAuthPolicy`] mTLS fingerprint map.
    pub fn with_tls_server_client_auth(
        mut self,
        policy: rvoip_sip_transport::transport::tls::TlsServerClientAuthConfig,
    ) -> Self {
        self.tls_server_client_auth = policy;
        self
    }

    /// Require every inbound SIP TLS peer to present a certificate chaining
    /// to `client_ca_path`.
    pub fn require_tls_client_certificate(
        self,
        client_ca_path: impl Into<std::path::PathBuf>,
    ) -> Self {
        self.with_tls_server_client_auth(
            rvoip_sip_transport::transport::tls::TlsServerClientAuthConfig::required(
                client_ca_path,
            ),
        )
    }

    /// Verify an inbound SIP TLS client certificate when one is presented,
    /// while allowing peers without a certificate to use another configured
    /// listener authentication mechanism.
    pub fn verify_optional_tls_client_certificate(
        self,
        client_ca_path: impl Into<std::path::PathBuf>,
    ) -> Self {
        self.with_tls_server_client_auth(
            rvoip_sip_transport::transport::tls::TlsServerClientAuthConfig::optional(
                client_ca_path,
            ),
        )
    }

    /// Configure SIP TLS for RFC 5626 registered-flow reuse.
    ///
    /// No TLS listener certificate/key is required because inbound requests
    /// are expected on the outbound registration flow.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SipContactMode};
    /// let config = Config::local("alice", 5060)
    ///     .tls_registered_flow_rfc5626("urn:uuid:00000000-0000-0000-0000-000000000001");
    /// assert_eq!(config.sip_contact_mode, SipContactMode::RegisteredFlowRfc5626);
    /// assert!(config.sip_outbound_enabled);
    /// ```
    pub fn tls_registered_flow_rfc5626(mut self, sip_instance: impl Into<String>) -> Self {
        self.sip_tls_mode = SipTlsMode::ClientOnly;
        self.sip_contact_mode = SipContactMode::RegisteredFlowRfc5626;
        self.sip_outbound_enabled = true;
        self.sip_instance = Some(sip_instance.into());
        self
    }

    /// Configure SIP TLS for PBX symmetric-transport registered-flow reuse.
    ///
    /// This mode keeps the registration flow alive but does not require the
    /// registrar to echo RFC 5626 Contact parameters.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SipContactMode};
    /// let config = Config::local("alice", 5060)
    ///     .tls_registered_flow_symmetric("urn:uuid:00000000-0000-0000-0000-000000000001");
    /// assert_eq!(config.sip_contact_mode, SipContactMode::RegisteredFlowSymmetric);
    /// ```
    pub fn tls_registered_flow_symmetric(mut self, sip_instance: impl Into<String>) -> Self {
        self.sip_tls_mode = SipTlsMode::ClientOnly;
        self.sip_contact_mode = SipContactMode::RegisteredFlowSymmetric;
        self.sip_instance = Some(sip_instance.into());
        self
    }

    /// Validate the SIP TLS/contact-mode configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let config = Config::local("alice", 5060);
    /// config.validate().unwrap();
    /// ```
    pub fn validate(&self) -> Result<()> {
        let effective_tls_mode = self.effective_tls_mode();

        if let Some(address) = self.sip_advertised_addr {
            if address.ip().is_unspecified() || address.port() == 0 {
                return Err(SessionError::ConfigError(
                    "sip_advertised_addr must have a non-unspecified IP and nonzero port"
                        .to_string(),
                ));
            }
        }
        if let Some(address) = self.tls_advertised_addr {
            if address.ip().is_unspecified() || address.port() == 0 {
                return Err(SessionError::ConfigError(
                    "tls_advertised_addr must have a non-unspecified IP and nonzero port"
                        .to_string(),
                ));
            }
        }
        if self
            .media_public_addr
            .is_some_and(|address| address.ip().is_unspecified())
        {
            return Err(SessionError::ConfigError(
                "media_public_addr must not use an unspecified IP".to_string(),
            ));
        }

        let tls_client_auth_enabled = self.tls_server_client_auth.mode
            != rvoip_sip_transport::transport::tls::TlsClientAuthMode::Disabled;
        if tls_client_auth_enabled {
            if !matches!(
                effective_tls_mode,
                SipTlsMode::ServerOnly | SipTlsMode::ClientAndServer
            ) {
                return Err(SessionError::ConfigError(
                    "inbound TLS client-certificate authentication requires a SIP TLS listener"
                        .to_string(),
                ));
            }
            if self
                .tls_server_client_auth
                .client_ca_path
                .as_deref()
                .is_none_or(|path| path.as_os_str().is_empty())
            {
                return Err(SessionError::ConfigError(
                    "inbound TLS client-certificate authentication requires an explicit client CA bundle"
                        .to_string(),
                ));
            }
        }

        if self.tls_cert_path.is_some() ^ self.tls_key_path.is_some() {
            return Err(SessionError::ConfigError(
                "TLS listener certificate and key must be provided together".to_string(),
            ));
        }
        if self.tls_client_cert_path.is_some() ^ self.tls_client_key_path.is_some() {
            return Err(SessionError::ConfigError(
                "TLS client certificate and key must be provided together".to_string(),
            ));
        }
        if self.registration_refresh_jitter_percent > 50 {
            return Err(SessionError::ConfigError(
                "registration_refresh_jitter_percent must be <= 50".to_string(),
            ));
        }
        if self.incoming_call_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "incoming_call_channel_capacity must be at least 1".to_string(),
            ));
        }
        if self.state_event_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "state_event_channel_capacity must be at least 1".to_string(),
            ));
        }
        if self.sip_transport_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "sip_transport_channel_capacity must be at least 1".to_string(),
            ));
        }
        if matches!(self.sip_transport_dispatch_workers, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transport_dispatch_workers must be at least 1 when set".to_string(),
            ));
        }
        if let Some(workers) = self.sip_transport_dispatch_workers {
            if workers
                > rvoip_sip_dialog::transaction::transport::MAX_TRANSPORT_EVENT_DISPATCH_WORKERS
            {
                return Err(SessionError::ConfigError(format!(
                    "sip_transport_dispatch_workers must be <= {} when set",
                    rvoip_sip_dialog::transaction::transport::MAX_TRANSPORT_EVENT_DISPATCH_WORKERS
                )));
            }
        }
        if matches!(self.sip_transport_dispatch_queue_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transport_dispatch_queue_capacity must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.sip_udp_recv_buffer_size, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_udp_recv_buffer_size must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.sip_udp_send_buffer_size, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_udp_send_buffer_size must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.sip_udp_parse_workers, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_udp_parse_workers must be at least 1 when set".to_string(),
            ));
        }
        if let Some(workers) = self.sip_udp_parse_workers {
            if workers > rvoip_sip_transport::UdpParseConfig::MAX_WORKERS {
                return Err(SessionError::ConfigError(format!(
                    "sip_udp_parse_workers must be <= {} when set",
                    rvoip_sip_transport::UdpParseConfig::MAX_WORKERS
                )));
            }
        }
        if matches!(self.sip_udp_parse_queue_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_udp_parse_queue_capacity must be at least 1 when set".to_string(),
            ));
        }
        if self.transaction_event_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "transaction_event_channel_capacity must be at least 1".to_string(),
            ));
        }
        if matches!(self.sip_transaction_dispatch_workers, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transaction_dispatch_workers must be at least 1 when set".to_string(),
            ));
        }
        if let Some(workers) = self.sip_transaction_dispatch_workers {
            if workers > rvoip_sip_dialog::transaction::MAX_TRANSACTION_DISPATCH_WORKERS {
                return Err(SessionError::ConfigError(format!(
                    "sip_transaction_dispatch_workers must be <= {} when set",
                    rvoip_sip_dialog::transaction::MAX_TRANSACTION_DISPATCH_WORKERS
                )));
            }
        }
        if matches!(self.sip_transaction_dispatch_queue_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transaction_dispatch_queue_capacity must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.sip_transaction_command_channel_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transaction_command_channel_capacity must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.sip_transaction_dispatch_priority_burst_max, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transaction_dispatch_priority_burst_max must be at least 1 when set"
                    .to_string(),
            ));
        }
        if matches!(self.sip_invite_2xx_retransmit_max_due_per_tick, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_invite_2xx_retransmit_max_due_per_tick must be at least 1 when set"
                    .to_string(),
            ));
        }
        if matches!(self.sip_dialog_dispatch_workers, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_dialog_dispatch_workers must be at least 1 when set".to_string(),
            ));
        }
        if let Some(workers) = self.sip_dialog_dispatch_workers {
            if workers > rvoip_sip_dialog::manager::MAX_DIALOG_EVENT_DISPATCH_WORKERS {
                return Err(SessionError::ConfigError(format!(
                    "sip_dialog_dispatch_workers must be <= {} when set",
                    rvoip_sip_dialog::manager::MAX_DIALOG_EVENT_DISPATCH_WORKERS
                )));
            }
        }
        if matches!(self.sip_dialog_dispatch_queue_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_dialog_dispatch_queue_capacity must be at least 1 when set".to_string(),
            ));
        }
        if self.global_event_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "global_event_channel_capacity must be at least 1".to_string(),
            ));
        }
        if self.session_event_dispatcher_workers == 0 {
            return Err(SessionError::ConfigError(
                "session_event_dispatcher_workers must be at least 1".to_string(),
            ));
        }
        if self.session_event_dispatcher_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "session_event_dispatcher_channel_capacity must be at least 1".to_string(),
            ));
        }
        if matches!(self.server_call_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "server_call_capacity must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.server_retained_lifecycle_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "server_retained_lifecycle_capacity must be at least 1 when set".to_string(),
            ));
        }
        match (
            self.server_call_capacity,
            self.server_retained_lifecycle_capacity,
        ) {
            (None, Some(_)) => {
                return Err(SessionError::ConfigError(
                    "server_retained_lifecycle_capacity requires server_call_capacity".to_string(),
                ));
            }
            (Some(active), Some(retained)) if retained < active => {
                return Err(SessionError::ConfigError(format!(
                    "server_retained_lifecycle_capacity ({retained}) must be >= server_call_capacity ({active})"
                )));
            }
            (None, None) | (Some(_), None) | (Some(_), Some(_)) => {}
        }
        if matches!(self.server_call_admission_limit, Some(0)) {
            return Err(SessionError::ConfigError(
                "server_call_admission_limit must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.server_call_admission_soft_limit, Some(0)) {
            return Err(SessionError::ConfigError(
                "server_call_admission_soft_limit must be at least 1 when set".to_string(),
            ));
        }
        if let (Some(soft), Some(hard)) = (
            self.server_call_admission_soft_limit,
            self.server_call_admission_limit,
        ) {
            if soft > hard {
                return Err(SessionError::ConfigError(format!(
                    "server_call_admission_soft_limit ({soft}) must be <= server_call_admission_limit ({hard})"
                )));
            }
        }
        if matches!(self.server_call_admission_pacing_delay_ms, Some(0)) {
            return Err(SessionError::ConfigError(
                "server_call_admission_pacing_delay_ms must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.server_overload_retry_after_secs, Some(0)) {
            return Err(SessionError::ConfigError(
                "server_overload_retry_after_secs must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.media_session_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "media_session_capacity must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.media_port_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "media_port_capacity must be at least 1 when set".to_string(),
            ));
        }
        if self.rtp_session_buffer_config.sender_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "rtp_session_buffer_config.sender_channel_capacity must be at least 1".to_string(),
            ));
        }
        if self.rtp_session_buffer_config.receiver_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "rtp_session_buffer_config.receiver_channel_capacity must be at least 1"
                    .to_string(),
            ));
        }
        if self.rtp_session_buffer_config.event_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "rtp_session_buffer_config.event_channel_capacity must be at least 1".to_string(),
            ));
        }
        if self.rtp_transport_buffer_config.event_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "rtp_transport_buffer_config.event_channel_capacity must be at least 1".to_string(),
            ));
        }
        if self.rtp_transport_buffer_config.recv_buffer_size == 0 {
            return Err(SessionError::ConfigError(
                "rtp_transport_buffer_config.recv_buffer_size must be at least 1".to_string(),
            ));
        }
        if self.rtp_transport_buffer_config.rtcp_recv_buffer_size == 0 {
            return Err(SessionError::ConfigError(
                "rtp_transport_buffer_config.rtcp_recv_buffer_size must be at least 1".to_string(),
            ));
        }
        if self
            .media_session_controller_config
            .audio_frame_pool
            .sample_rate
            == 0
        {
            return Err(SessionError::ConfigError(
                "media_session_controller_config.audio_frame_pool.sample_rate must be at least 1"
                    .to_string(),
            ));
        }
        if self
            .media_session_controller_config
            .audio_frame_pool
            .channels
            == 0
        {
            return Err(SessionError::ConfigError(
                "media_session_controller_config.audio_frame_pool.channels must be at least 1"
                    .to_string(),
            ));
        }
        if self
            .media_session_controller_config
            .audio_frame_pool
            .samples_per_frame
            == 0
        {
            return Err(SessionError::ConfigError(
                "media_session_controller_config.audio_frame_pool.samples_per_frame must be at least 1"
                    .to_string(),
            ));
        }
        if self.media_session_controller_config.rtp_buffer_size == 0 {
            return Err(SessionError::ConfigError(
                "media_session_controller_config.rtp_buffer_size must be at least 1".to_string(),
            ));
        }
        if self.media_session_controller_config.rtp_buffer_max_count == 0 {
            return Err(SessionError::ConfigError(
                "media_session_controller_config.rtp_buffer_max_count must be at least 1"
                    .to_string(),
            ));
        }
        if self
            .media_session_controller_config
            .rtp_buffer_initial_count
            > self.media_session_controller_config.rtp_buffer_max_count
        {
            return Err(SessionError::ConfigError(
                "media_session_controller_config.rtp_buffer_initial_count must be <= rtp_buffer_max_count"
                    .to_string(),
            ));
        }
        #[cfg(feature = "perf-tests")]
        if let Some(limit) = self.perf_max_rss_growth_mb_per_hr {
            if !limit.is_finite() || limit <= 0.0 {
                return Err(SessionError::ConfigError(
                    "perf_max_rss_growth_mb_per_hr must be finite and greater than 0".to_string(),
                ));
            }
        }
        if self.media_port_start < MIN_PORT {
            return Err(SessionError::ConfigError(format!(
                "media_port_start must be >= {}",
                MIN_PORT
            )));
        }
        if self.media_port_start > self.media_port_end {
            return Err(SessionError::ConfigError(
                "media_port_start must be <= media_port_end".to_string(),
            ));
        }
        if let Some(capacity) = self.media_port_capacity {
            let available = self.media_port_end as usize - self.media_port_start as usize + 1;
            if available < capacity {
                return Err(SessionError::ConfigError(format!(
                    "media port range {}-{} provides {} ports, below requested media_port_capacity {}",
                    self.media_port_start, self.media_port_end, available, capacity
                )));
            }
        }
        if let MediaMode::SignalingOnly { sdp_rtp_port: 0 } = self.media_mode {
            return Err(SessionError::ConfigError(
                "signaling-only media SDP RTP port must be at least 1".to_string(),
            ));
        }
        validate_beta_media_codecs(&self.offered_codecs, self.comfort_noise_enabled)?;
        if self.srtp_required && !self.offer_srtp {
            return Err(SessionError::ConfigError(
                "srtp_required=true requires offer_srtp=true".to_string(),
            ));
        }
        if self.offer_srtp && self.srtp_offered_suites.is_empty() {
            return Err(SessionError::ConfigError(
                "offer_srtp=true requires at least one srtp_offered_suites entry".to_string(),
            ));
        }
        if matches!(
            effective_tls_mode,
            SipTlsMode::ServerOnly | SipTlsMode::ClientAndServer
        ) && (self.tls_cert_path.is_none() || self.tls_key_path.is_none())
        {
            return Err(SessionError::ConfigError(
                "SIP TLS listener modes require tls_cert_path and tls_key_path".to_string(),
            ));
        }

        match self.sip_contact_mode {
            SipContactMode::ReachableContact => match effective_tls_mode {
                SipTlsMode::Disabled => {}
                SipTlsMode::ClientOnly => {
                    if self.contact_uri.is_none() {
                        return Err(SessionError::ConfigError(
                            "reachable TLS Contact mode with ClientOnly requires an explicit external contact_uri".to_string(),
                        ));
                    }
                }
                SipTlsMode::ServerOnly | SipTlsMode::ClientAndServer => {
                    if self.tls_bind_addr.is_none() {
                        return Err(SessionError::ConfigError(
                            "reachable TLS Contact mode requires tls_bind_addr".to_string(),
                        ));
                    }
                    if self.tls_cert_path.is_none() || self.tls_key_path.is_none() {
                        return Err(SessionError::ConfigError(
                            "reachable TLS Contact mode requires tls_cert_path and tls_key_path"
                                .to_string(),
                        ));
                    }
                }
            },
            SipContactMode::RegisteredFlowRfc5626 => {
                if !matches!(
                    effective_tls_mode,
                    SipTlsMode::ClientOnly | SipTlsMode::ClientAndServer
                ) {
                    return Err(SessionError::ConfigError(
                        "RFC 5626 registered-flow mode requires SIP TLS ClientOnly or ClientAndServer".to_string(),
                    ));
                }
                if !self.sip_outbound_enabled {
                    return Err(SessionError::ConfigError(
                        "RFC 5626 registered-flow mode requires sip_outbound_enabled=true"
                            .to_string(),
                    ));
                }
                if self
                    .sip_instance
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    return Err(SessionError::ConfigError(
                        "RFC 5626 registered-flow mode requires a stable sip_instance URN"
                            .to_string(),
                    ));
                }
            }
            SipContactMode::RegisteredFlowSymmetric => {
                if !matches!(
                    effective_tls_mode,
                    SipTlsMode::ClientOnly | SipTlsMode::ClientAndServer
                ) {
                    return Err(SessionError::ConfigError(
                        "symmetric registered-flow mode requires SIP TLS ClientOnly or ClientAndServer".to_string(),
                    ));
                }
            }
        }

        // SIP_API_DESIGN_2 §7.4 — auto_emit_extra_headers stack-managed
        // rejection. The state machine's auto-emit paths can't route
        // through `HeaderPolicy::validate_outbound` (there's no
        // builder), so we hard-fail at Config-construction time when
        // an application stages a name that would desync the dialog
        // or transaction.
        for header in &self.auto_emit_extra_headers {
            if crate::api::headers::policy::forbidden_for_carry_through(&header.name()) {
                return Err(SessionError::ConfigError(format!(
                    "Config.auto_emit_extra_headers contains stack-managed header {:?} \
                     (Call-ID / CSeq / Via / Max-Forwards / Content-Length / Record-Route / \
                     Route are owned by the dialog/transaction layer)",
                    header.name()
                )));
            }
        }

        Ok(())
    }

    fn effective_tls_mode(&self) -> SipTlsMode {
        if self.sip_tls_mode == SipTlsMode::Disabled
            && self.tls_cert_path.is_some()
            && self.tls_key_path.is_some()
        {
            SipTlsMode::ClientAndServer
        } else {
            self.sip_tls_mode
        }
    }
}

fn validate_beta_media_codecs(offered_codecs: &[u8], comfort_noise_enabled: bool) -> Result<()> {
    if offered_codecs.is_empty() {
        return Err(SessionError::ConfigError(
            "offered_codecs must include at least one beta-supported audio codec".to_string(),
        ));
    }

    let supported_payloads = match (cfg!(feature = "g729"), cfg!(feature = "opus")) {
        (true, true) => "0, 8, 18, 111, 101, and 13 when comfort_noise_enabled=true",
        (true, false) => "0, 8, 18, 101, and 13 when comfort_noise_enabled=true",
        (false, true) => "0, 8, 111, 101, and 13 when comfort_noise_enabled=true",
        (false, false) => "0, 8, 101, and 13 when comfort_noise_enabled=true",
    };
    let mut has_audio = false;
    let mut seen = std::collections::BTreeSet::new();
    for &pt in offered_codecs {
        if !seen.insert(pt) {
            return Err(SessionError::ConfigError(format!(
                "offered_codecs contains duplicate payload type {}",
                pt
            )));
        }

        match pt {
            0 | 8 => has_audio = true,
            18 => {
                #[cfg(feature = "g729")]
                {
                    has_audio = true;
                }
                #[cfg(not(feature = "g729"))]
                {
                    return Err(SessionError::ConfigError(
                        "payload type 18 requires the rvoip-sip `g729` feature".to_string(),
                    ));
                }
            }
            111 => {
                #[cfg(feature = "opus")]
                {
                    has_audio = true;
                }
                #[cfg(not(feature = "opus"))]
                {
                    return Err(SessionError::ConfigError(
                        "payload type 111 requires the rvoip-sip `opus` feature".to_string(),
                    ));
                }
            }
            101 => {}
            13 if comfort_noise_enabled => {}
            13 => {
                return Err(SessionError::ConfigError(
                    "payload type 13 requires comfort_noise_enabled=true".to_string(),
                ));
            }
            unsupported => {
                return Err(SessionError::ConfigError(format!(
                    "payload type {} is not beta-supported for full media; supported payloads are {}",
                    unsupported, supported_payloads
                )));
            }
        }
    }

    if !has_audio {
        return Err(SessionError::ConfigError(
            match (cfg!(feature = "g729"), cfg!(feature = "opus")) {
                (true, true) => "offered_codecs must include PCMU (0), PCMA (8), G.729 (18), or Opus (111) for beta full-media support",
                (true, false) => "offered_codecs must include PCMU (0), PCMA (8), or G.729 (18) for beta full-media support",
                (false, true) => "offered_codecs must include PCMU (0), PCMA (8), or Opus (111) for beta full-media support",
                (false, false) => "offered_codecs must include PCMU (0) or PCMA (8) for beta full-media support",
            }
            .to_string(),
        ));
    }

    Ok(())
}

impl Default for Config {
    fn default() -> Self {
        Config::local("user", 5060)
    }
}

#[cfg(test)]
mod config_tests {
    use super::{
        complete_established_bye_dispatch, exact_terminal_completion_result,
        run_bounded_exact_response_batch, Config, ExactResponseRegistration,
        ExactResponseRetryCause, OobAuthRetry, PendingExactResponseRegistry, Registration,
        RegistrationHandle, RegistrationInfo, RegistrationStatus, SetupTeardownDeadline,
        SetupTeardownDeadlineQueue, SetupTeardownDeadlineScheduler, SetupTeardownWatchdogKind,
        UnifiedCoordinator, EXACT_RESPONSE_MAX_RETRIES, EXACT_RESPONSE_RETRY_DELAY,
        EXACT_RESPONSE_SLOW_RETRY_DELAY, SETUP_TEARDOWN_TIMEOUT_CONCURRENCY,
    };
    use crate::api::handle::CallId;
    use crate::api::incoming::ExactResponseObligation;
    use crate::api::lifecycle::{ExactTerminalClaim, ExactTerminalCompletion};
    use crate::api::trace_redactor::{RedactionDecision, TraceRedactor};
    use crate::errors::SessionError;
    use crate::session_store::SessionStore;
    use crate::state_table::types::{Role, SessionId};
    use rvoip_sip_core::types::headers::HeaderName;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    struct CanaryTracePolicy(&'static str);

    #[test]
    fn exact_response_registry_retains_one_owner_and_rejects_key_collision() {
        let registry = PendingExactResponseRegistry::default();
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-registry-owner".into(),
            rvoip_sip_core::Method::Info,
            true,
        );
        let owner = Arc::new(ExactResponseObligation::new(
            CallId::new(),
            transaction.clone(),
        ));
        assert!(matches!(
            registry.register(Arc::clone(&owner)),
            ExactResponseRegistration::Registered
        ));
        drop(owner);
        assert_eq!(registry.len(), 1, "registry did not retain exact owner");

        let collision = Arc::new(ExactResponseObligation::new(CallId::new(), transaction));
        assert!(matches!(
            registry.register(collision),
            ExactResponseRegistration::Collision
        ));
        registry.begin_close();
        registry.clear();
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn exact_response_busy_and_timeout_do_not_consume_zero_wire_retry_budget() {
        let registry = PendingExactResponseRegistry::default();
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-registry-retry-budget".into(),
            rvoip_sip_core::Method::Info,
            true,
        );
        let owner = Arc::new(ExactResponseObligation::new(
            CallId::new(),
            transaction.clone(),
        ));
        assert!(matches!(
            registry.register(owner),
            ExactResponseRegistration::Registered
        ));

        for _ in 0..(usize::from(EXACT_RESPONSE_MAX_RETRIES) * 2) {
            let plan = registry.retry_plan(&transaction, ExactResponseRetryCause::BusyOrTimeout);
            assert_eq!(plan.delay, EXACT_RESPONSE_RETRY_DELAY);
            assert!(!plan.slow_path);
        }
        assert!(
            !registry.retry_attempts.contains_key(&transaction),
            "busy/timeout consumed the zero-wire retry budget"
        );

        for _ in 0..EXACT_RESPONSE_MAX_RETRIES {
            let plan = registry.retry_plan(&transaction, ExactResponseRetryCause::ZeroWire);
            assert_eq!(plan.delay, EXACT_RESPONSE_RETRY_DELAY);
            assert!(!plan.slow_path);
        }
        let slow = registry.retry_plan(&transaction, ExactResponseRetryCause::ZeroWire);
        assert_eq!(slow.delay, EXACT_RESPONSE_SLOW_RETRY_DELAY);
        assert!(slow.slow_path);
        assert_eq!(registry.len(), 1, "slow retry removed the obligation");
    }

    #[tokio::test]
    async fn exact_response_due_batch_uses_bounded_parallelism() {
        const ITEMS: usize = 96;
        const CONCURRENCY: usize = 24;

        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let batch = {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            let gate = Arc::clone(&gate);
            tokio::spawn(async move {
                run_bounded_exact_response_batch((0..ITEMS).collect(), CONCURRENCY, move |_| {
                    let active = Arc::clone(&active);
                    let peak = Arc::clone(&peak);
                    let gate = Arc::clone(&gate);
                    async move {
                        let current = active.fetch_add(1, Ordering::AcqRel) + 1;
                        peak.fetch_max(current, Ordering::AcqRel);
                        let permit = gate.acquire().await.expect("batch gate open");
                        permit.forget();
                        active.fetch_sub(1, Ordering::AcqRel);
                    }
                })
                .await;
            })
        };

        tokio::time::timeout(Duration::from_secs(1), async {
            while active.load(Ordering::Acquire) != CONCURRENCY {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("bounded batch did not fill its concurrency window");
        assert_eq!(peak.load(Ordering::Acquire), CONCURRENCY);

        gate.add_permits(ITEMS);
        tokio::time::timeout(Duration::from_secs(1), batch)
            .await
            .expect("bounded batch did not drain")
            .expect("bounded batch task panicked");
        assert_eq!(active.load(Ordering::Acquire), 0);
        assert_eq!(peak.load(Ordering::Acquire), CONCURRENCY);
    }

    #[test]
    fn observational_terminal_publication_failure_does_not_fail_protocol_completion() {
        assert!(
            exact_terminal_completion_result(ExactTerminalCompletion::PublicationFailed).is_ok()
        );
        assert!(exact_terminal_completion_result(ExactTerminalCompletion::ReleaseFailed).is_err());
        assert!(exact_terminal_completion_result(
            ExactTerminalCompletion::PublicationAndReleaseFailed
        )
        .is_err());
    }

    #[tokio::test]
    async fn post_send_bye_dispatch_race_joins_exact_success_confirmation() {
        let result = complete_established_bye_dispatch(
            Err(SessionError::Other(
                "lower-layer operation failed (class=opaque-erased)".to_string(),
            )),
            true,
            std::future::ready(Ok(())),
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn post_send_bye_dispatch_race_does_not_swallow_wire_failure() {
        let result = complete_established_bye_dispatch(
            Err(SessionError::Other(
                "lower-layer operation failed (class=opaque-erased)".to_string(),
            )),
            true,
            std::future::ready(Err(SessionError::ProtocolError(
                "SIP BYE received a non-success final response".to_string(),
            ))),
        )
        .await;

        assert!(matches!(result, Err(SessionError::ProtocolError(_))));
    }

    #[tokio::test]
    async fn pre_send_bye_dispatch_failure_preserves_original_error() {
        let confirmation_polled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let confirmation_probe = Arc::clone(&confirmation_polled);
        let confirmation = async move {
            confirmation_probe.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        };
        let result = complete_established_bye_dispatch(
            Err(SessionError::InvalidTransition(
                "BYE was not dispatched".to_string(),
            )),
            false,
            confirmation,
        )
        .await;

        assert!(matches!(result, Err(SessionError::InvalidTransition(_))));
        assert!(!confirmation_polled.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[tokio::test]
    async fn setup_teardown_deadlines_are_compact_ordered_and_bounded() {
        let store = SessionStore::with_capacity(4);
        let mut handles = Vec::new();
        for _ in 0..3 {
            let session_id = SessionId::new();
            store
                .create_session(session_id.clone(), Role::UAC, false)
                .await
                .expect("create exact timeout fixture");
            handles.push(
                store
                    .lifecycle_handle(&session_id)
                    .expect("fixture lifecycle handle"),
            );
        }

        let now = Instant::now();
        let entered_state_at = now;
        let mut queue = SetupTeardownDeadlineQueue::default();
        assert!(queue.push(
            now + Duration::from_secs(3),
            handles[0].clone(),
            entered_state_at,
            SetupTeardownWatchdogKind::OutboundSetup,
        ));
        assert!(queue.push(
            now + Duration::from_secs(1),
            handles[1].clone(),
            entered_state_at,
            SetupTeardownWatchdogKind::InboundSetup,
        ));
        assert!(!queue.push(
            now + Duration::from_secs(2),
            handles[2].clone(),
            entered_state_at,
            SetupTeardownWatchdogKind::Cancellation,
        ));

        assert_eq!(queue.next_deadline(), Some(now + Duration::from_secs(1)));
        let first = queue.take_due(now + Duration::from_secs(2), 1);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, SetupTeardownWatchdogKind::InboundSetup);
        let second = queue.take_due(now + Duration::from_secs(2), 8);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].kind, SetupTeardownWatchdogKind::Cancellation);
        assert_eq!(queue.drain().len(), 1);

        assert!(
            std::mem::size_of::<SetupTeardownDeadline>() <= 128,
            "deadline records must stay compact"
        );
        let scheduler = SetupTeardownDeadlineScheduler::default();
        assert_eq!(
            scheduler.fire_slots.available_permits(),
            SETUP_TEARDOWN_TIMEOUT_CONCURRENCY
        );
    }

    #[tokio::test]
    async fn retained_media_watchdog_sleep_wakes_and_joins_on_close() {
        let scheduler = Arc::new(SetupTeardownDeadlineScheduler::default());
        let task_scheduler = Arc::clone(&scheduler);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        assert!(scheduler.spawn_lifecycle_task(async move {
            let _ = started_tx.send(());
            let expired = task_scheduler
                .sleep_or_closed(Duration::from_secs(60))
                .await;
            let _ = result_tx.send(expired);
        }));
        started_rx.await.expect("media watchdog sleep started");

        scheduler
            .close_and_wait(Duration::from_secs(1))
            .await
            .expect("close woke and joined retained media watchdog");
        assert!(!result_rx.await.expect("watchdog returned close outcome"));
        assert_eq!(scheduler.tasks.count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn earlier_deadline_wakes_runner_without_waking_media_waiter() {
        let coordinator = UnifiedCoordinator::new(Config::local("split-watchdog-notifies", 0))
            .await
            .expect("start split-notify coordinator");
        let store = &coordinator.helpers.state_machine.store;

        let late = SessionId::new();
        store
            .create_session(late.clone(), Role::UAC, false)
            .await
            .expect("create late-deadline session");
        let late_entered_at = Instant::now();
        store
            .update_session_with(&late, |session| {
                session.call_state = crate::types::CallState::Initiating;
                session.entered_state_at = late_entered_at;
            })
            .await
            .expect("arm late-deadline state");
        let late_handle = store
            .lifecycle_handle(&late)
            .expect("late-deadline exact handle");
        let late_deadline = Instant::now() + Duration::from_secs(60);
        assert!(coordinator.setup_teardown_scheduler.schedule(
            late_deadline,
            late_handle,
            late_entered_at,
            SetupTeardownWatchdogKind::OutboundSetup,
        ));
        tokio::time::timeout(
            Duration::from_secs(2),
            coordinator
                .setup_teardown_scheduler
                .wait_for_runner_deadline_for_test(late_deadline),
        )
        .await
        .expect("runner began sleeping on late deadline");

        let media_scheduler = Arc::clone(&coordinator.setup_teardown_scheduler);
        let task_scheduler = Arc::clone(&media_scheduler);
        let (media_started_tx, media_started_rx) = tokio::sync::oneshot::channel();
        let (media_result_tx, mut media_result_rx) = tokio::sync::oneshot::channel();
        assert!(media_scheduler.spawn_lifecycle_task(async move {
            let _ = media_started_tx.send(());
            let expired = task_scheduler
                .sleep_or_closed(Duration::from_secs(60))
                .await;
            let _ = media_result_tx.send(expired);
        }));
        media_started_rx.await.expect("media waiter started");

        let early = SessionId::new();
        store
            .create_session(early.clone(), Role::UAC, false)
            .await
            .expect("create early-deadline session");
        let early_entered_at = Instant::now();
        store
            .update_session_with(&early, |session| {
                session.call_state = crate::types::CallState::Initiating;
                session.entered_state_at = early_entered_at;
            })
            .await
            .expect("arm early-deadline state");
        let early_handle = store
            .lifecycle_handle(&early)
            .expect("early-deadline exact handle");
        let mut early_lifecycle = coordinator.lifecycle_watcher(&early);
        assert!(coordinator.setup_teardown_scheduler.schedule(
            Instant::now() + Duration::from_millis(50),
            early_handle.clone(),
            early_entered_at,
            SetupTeardownWatchdogKind::OutboundSetup,
        ));

        tokio::time::timeout(Duration::from_secs(2), early_lifecycle.changed())
            .await
            .expect("earlier deadline fired while runner had a later timer")
            .expect("early lifecycle remained observable");
        let completion = match coordinator
            .app_event_publisher
            .claim_exact_terminal(&early_handle)
        {
            ExactTerminalClaim::Observer(observer) => observer.wait().await,
            ExactTerminalClaim::Owner(_) => {
                panic!("earlier deadline must already own exact terminal release")
            }
        };
        exact_terminal_completion_result(completion)
            .expect("earlier deadline exact release completed");
        assert!(matches!(
            media_result_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));

        coordinator
            .shutdown_gracefully(Some(Duration::ZERO))
            .await
            .expect("split-notify coordinator shut down");
        assert!(!media_result_rx.await.expect("media waiter observed close"));
        assert!(
            store.lifecycle_handle(&late).is_some(),
            "later deadline must be drained rather than fired"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shared_setup_teardown_scheduler_fires_disarms_and_drains_exactly() {
        let coordinator = UnifiedCoordinator::new(
            Config::local("shared-watchdog-scheduler", 0).with_setup_teardown_timeout_secs(1),
        )
        .await
        .expect("start watchdog test coordinator");
        let store = &coordinator.helpers.state_machine.store;

        let firing = SessionId::new();
        store
            .create_session(firing.clone(), Role::UAC, false)
            .await
            .expect("create firing session");
        store
            .update_session_with(&firing, |session| {
                session.call_state = crate::types::CallState::Initiating;
                session.entered_state_at = Instant::now();
            })
            .await
            .expect("arm firing state");
        let firing_handle = store
            .lifecycle_handle(&firing)
            .expect("firing fixture handle");
        let mut firing_lifecycle = coordinator.lifecycle_watcher(&firing);
        coordinator
            .schedule_setup_teardown_timeout_if_current(
                &firing,
                SetupTeardownWatchdogKind::OutboundSetup,
            )
            .await;

        let changed = SessionId::new();
        store
            .create_session(changed.clone(), Role::UAC, false)
            .await
            .expect("create changed-state session");
        store
            .update_session_with(&changed, |session| {
                session.call_state = crate::types::CallState::Initiating;
                session.entered_state_at = Instant::now();
            })
            .await
            .expect("arm changed-state session");
        coordinator
            .schedule_setup_teardown_timeout_if_current(
                &changed,
                SetupTeardownWatchdogKind::OutboundSetup,
            )
            .await;
        store
            .update_session_with(&changed, |session| {
                session.call_state = crate::types::CallState::Active;
                session.entered_state_at = Instant::now();
            })
            .await
            .expect("advance changed-state session");

        let stale = SessionId::new();
        store
            .create_session(stale.clone(), Role::UAC, false)
            .await
            .expect("create stale-handle session");
        let stale_handle = store
            .lifecycle_handle(&stale)
            .expect("stale fixture current handle")
            .with_next_slot_revision_for_test();
        assert!(coordinator.setup_teardown_scheduler.schedule(
            Instant::now() + Duration::from_millis(100),
            stale_handle,
            Instant::now(),
            SetupTeardownWatchdogKind::OutboundSetup,
        ));

        tokio::time::timeout(Duration::from_secs(4), firing_lifecycle.changed())
            .await
            .expect("firing watchdog lifecycle deadline")
            .expect("firing watchdog lifecycle remains observable");
        let completion = match coordinator
            .app_event_publisher
            .claim_exact_terminal(&firing_handle)
        {
            ExactTerminalClaim::Observer(observer) => {
                tokio::time::timeout(Duration::from_secs(4), observer.wait())
                    .await
                    .expect("firing watchdog exact release deadline")
            }
            ExactTerminalClaim::Owner(_) => {
                panic!("terminal lifecycle publication must already own the exact release")
            }
        };
        exact_terminal_completion_result(completion)
            .expect("firing watchdog exact release must complete");
        assert_eq!(coordinator.setup_teardown_scheduler.len(), 0);
        assert!(
            store.lifecycle_handle(&firing).is_none(),
            "the one current deadline must fire and release its exact lifetime"
        );
        assert!(
            store.lifecycle_handle(&changed).is_some(),
            "a state/revision change must disarm instead of firing"
        );
        assert!(
            store.lifecycle_handle(&stale).is_some(),
            "a stale exact handle must not target the current lifetime"
        );

        let shutdown = SessionId::new();
        store
            .create_session(shutdown.clone(), Role::UAC, false)
            .await
            .expect("create shutdown session");
        let shutdown_handle = store
            .lifecycle_handle(&shutdown)
            .expect("shutdown fixture handle");
        assert!(coordinator.setup_teardown_scheduler.schedule(
            Instant::now() + Duration::from_secs(60),
            shutdown_handle,
            Instant::now(),
            SetupTeardownWatchdogKind::OutboundSetup,
        ));
        coordinator.shutdown();
        let shutdown_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while coordinator.setup_teardown_scheduler.len() != 0
            && tokio::time::Instant::now() < shutdown_deadline
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(coordinator.setup_teardown_scheduler.len(), 0);
        assert!(
            store.lifecycle_handle(&shutdown).is_some(),
            "shutdown must drain a future deadline without firing it"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn graceful_shutdown_joins_claimed_watchdog_before_stopping_dependencies() {
        let coordinator = UnifiedCoordinator::new(Config::local("watchdog-drain-join", 0))
            .await
            .expect("start watchdog drain coordinator");
        let store = &coordinator.helpers.state_machine.store;
        let session_id = SessionId::new();
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create firing session");
        let entered_state_at = Instant::now();
        store
            .update_session_with(&session_id, |session| {
                session.call_state = crate::types::CallState::Initiating;
                session.entered_state_at = entered_state_at;
            })
            .await
            .expect("arm firing state");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("firing session exact handle");

        coordinator
            .setup_teardown_scheduler
            .pause_next_fire_for_test();
        assert!(coordinator.setup_teardown_scheduler.schedule(
            Instant::now(),
            handle,
            entered_state_at,
            SetupTeardownWatchdogKind::OutboundSetup,
        ));
        tokio::time::timeout(
            Duration::from_secs(2),
            coordinator
                .setup_teardown_scheduler
                .wait_for_paused_fire_for_test(),
        )
        .await
        .expect("watchdog fire reached deterministic pause");

        let first_coordinator = Arc::clone(&coordinator);
        let first_shutdown = tokio::spawn(async move {
            first_coordinator
                .shutdown_gracefully(Some(Duration::ZERO))
                .await
        });
        let second_coordinator = Arc::clone(&coordinator);
        let second_shutdown = tokio::spawn(async move {
            second_coordinator
                .shutdown_gracefully(Some(Duration::ZERO))
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while coordinator.setup_teardown_scheduler.is_accepting_for_test() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown established scheduler close boundary");
        assert!(!first_shutdown.is_finished());
        assert!(!second_shutdown.is_finished());

        coordinator
            .setup_teardown_scheduler
            .resume_paused_fire_for_test();
        tokio::time::timeout(Duration::from_secs(4), first_shutdown)
            .await
            .expect("first graceful shutdown joined watchdog")
            .expect("first shutdown task joined")
            .expect("first graceful shutdown succeeded");
        tokio::time::timeout(Duration::from_secs(4), second_shutdown)
            .await
            .expect("second graceful shutdown joined shared attempt")
            .expect("second shutdown task joined")
            .expect("second graceful shutdown succeeded");
        assert!(
            store.lifecycle_handle(&session_id).is_none(),
            "graceful shutdown must wait through exact terminal release"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn graceful_shutdown_drains_future_deadline_and_rejects_new_schedule() {
        let coordinator = UnifiedCoordinator::new(Config::local("watchdog-future-drain", 0))
            .await
            .expect("start future-drain coordinator");
        let store = &coordinator.helpers.state_machine.store;
        let session_id = SessionId::new();
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create future-deadline session");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("future-deadline exact handle");
        assert!(coordinator.setup_teardown_scheduler.schedule(
            Instant::now() + Duration::from_secs(60),
            handle.clone(),
            Instant::now(),
            SetupTeardownWatchdogKind::OutboundSetup,
        ));

        coordinator
            .shutdown_gracefully(Some(Duration::ZERO))
            .await
            .expect("graceful shutdown drained future deadline");
        assert_eq!(coordinator.setup_teardown_scheduler.len(), 0);
        assert!(
            store.lifecycle_handle(&session_id).is_some(),
            "a future deadline must be disarmed without firing"
        );
        assert!(!coordinator.setup_teardown_scheduler.schedule(
            Instant::now(),
            handle,
            Instant::now(),
            SetupTeardownWatchdogKind::OutboundSetup,
        ));
        assert_eq!(coordinator.setup_teardown_scheduler.len(), 0);
    }

    #[tokio::test]
    async fn hangup_without_exact_lifetime_fails_before_raw_id_reuse() {
        let coordinator = UnifiedCoordinator::new(Config::local("hangup-reuse-fence", 0))
            .await
            .expect("start hangup reuse coordinator");
        let reused = SessionId("hangup-reuse-fence-session".to_string());
        assert!(matches!(
            coordinator.hangup(&reused).await,
            Err(SessionError::SessionNotFound(_))
        ));

        coordinator
            .helpers
            .state_machine
            .store
            .create_session(reused.clone(), Role::UAC, false)
            .await
            .expect("reuse raw id after rejected hangup");
        tokio::task::yield_now().await;
        assert!(
            coordinator
                .helpers
                .state_machine
                .store
                .lifecycle_handle(&reused)
                .is_some(),
            "rejected raw-id hangup must not leave detached work targeting a later lifetime"
        );
        coordinator
            .shutdown_gracefully(Some(Duration::ZERO))
            .await
            .expect("hangup reuse coordinator shut down");
    }

    impl std::fmt::Debug for CanaryTracePolicy {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_tuple("CanaryTracePolicy")
                .field(&self.0)
                .finish()
        }
    }

    impl TraceRedactor for CanaryTracePolicy {
        fn redact(&self, _header: &HeaderName, _value: &str) -> RedactionDecision {
            RedactionDecision::Keep
        }
    }

    #[test]
    fn transaction_command_channel_default_is_small_and_configurable() {
        assert_eq!(Config::DEFAULT_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY, 32);
        let config =
            Config::local("alice", 5060).with_sip_transaction_command_channel_capacity(256);
        assert_eq!(config.sip_transaction_command_channel_capacity, Some(256));
        config.validate().expect("valid command channel capacity");
    }

    #[test]
    fn inbound_tls_client_auth_is_disabled_by_default_and_validated_when_enabled() {
        use rvoip_sip_transport::transport::tls::{TlsClientAuthMode, TlsServerClientAuthConfig};

        let default = Config::local("alice", 5060);
        assert_eq!(
            default.tls_server_client_auth.mode,
            TlsClientAuthMode::Disabled
        );

        let no_listener = Config::local("alice", 5060)
            .require_tls_client_certificate("client-ca.pem")
            .validate()
            .expect_err("client auth without a TLS listener must fail");
        assert!(matches!(
            no_listener,
            SessionError::ConfigError(ref detail) if detail.contains("requires a SIP TLS listener")
        ));

        let missing_ca = Config::local("alice", 5060)
            .tls_reachable_contact(
                "127.0.0.1:5061".parse().unwrap(),
                "server.pem",
                "server-key.pem",
            )
            .with_tls_server_client_auth(TlsServerClientAuthConfig {
                mode: TlsClientAuthMode::Required,
                client_ca_path: None,
            })
            .validate()
            .expect_err("enabled client auth without a CA must fail");
        assert!(matches!(
            missing_ca,
            SessionError::ConfigError(ref detail) if detail.contains("client CA bundle")
        ));

        let valid = Config::local("alice", 5060)
            .tls_reachable_contact(
                "127.0.0.1:5061".parse().unwrap(),
                "server.pem",
                "server-key.pem",
            )
            .verify_optional_tls_client_certificate("client-ca.pem");
        valid.validate().expect("complete inbound client auth");
        assert_eq!(
            valid.tls_server_client_auth.mode,
            TlsClientAuthMode::Optional
        );
    }

    #[test]
    fn transaction_command_channel_capacity_rejects_zero() {
        let error = Config::local("alice", 5060)
            .with_sip_transaction_command_channel_capacity(0)
            .validate()
            .expect_err("zero transaction command channel capacity should fail");
        assert!(matches!(
            error,
            SessionError::ConfigError(ref detail)
                if detail.contains("sip_transaction_command_channel_capacity")
        ));
    }

    #[test]
    fn high_cps_profile_sets_tunable_transaction_command_capacity() {
        let config = Config::local("alice", 5060).with_high_cps_udp_auto_answer(8000);
        assert_eq!(config.sip_transaction_command_channel_capacity, Some(1000));

        let overridden = Config::local("alice", 5060)
            .with_sip_transaction_command_channel_capacity(64)
            .with_high_cps_udp_auto_answer(8000);
        assert_eq!(
            overridden.sip_transaction_command_channel_capacity,
            Some(64)
        );
    }

    #[test]
    fn g729_annex_b_defaults_enabled_and_is_configurable() {
        let config = Config::local("alice", 5060);
        assert!(config.g729_annex_b);

        let config = Config::local("alice", 5060).with_g729_annex_b(false);
        assert!(!config.g729_annex_b);
    }

    #[cfg(feature = "g729")]
    #[test]
    fn g729_payload_is_valid_when_feature_enabled() {
        let mut config = Config::local("alice", 5060);
        config.offered_codecs = vec![18, 101];
        config.validate().expect("g729 feature should allow PT18");
    }

    #[cfg(not(feature = "g729"))]
    #[test]
    fn g729_payload_requires_feature() {
        let mut config = Config::local("alice", 5060);
        config.offered_codecs = vec![18, 101];
        let error = config
            .validate()
            .expect_err("PT18 should require the g729 feature");
        assert!(matches!(
            error,
            SessionError::ConfigError(ref detail) if detail.contains("`g729` feature")
        ));
    }

    #[cfg(feature = "opus")]
    #[test]
    fn opus_payload_is_valid_when_feature_enabled() {
        let mut config = Config::local("alice", 5060);
        config.offered_codecs = vec![111, 101];
        config.validate().expect("opus feature should allow PT111");
    }

    #[cfg(not(feature = "opus"))]
    #[test]
    fn opus_payload_requires_feature() {
        let mut config = Config::local("alice", 5060);
        config.offered_codecs = vec![111, 101];
        let error = config
            .validate()
            .expect_err("PT111 should require the opus feature");
        assert!(matches!(
            error,
            SessionError::ConfigError(ref detail) if detail.contains("`opus` feature")
        ));
    }

    #[test]
    fn active_call_media_watchdogs_are_disabled_by_default_and_configurable() {
        let config = Config::local("alice", 5060);
        assert_eq!(config.active_call_no_media_timeout_secs, 0);
        assert_eq!(config.active_call_media_idle_timeout_secs, 0);

        let config = config
            .with_active_call_no_media_timeout_secs(60)
            .with_active_call_media_idle_timeout_secs(90);
        assert_eq!(config.active_call_no_media_timeout_secs, 60);
        assert_eq!(config.active_call_media_idle_timeout_secs, 90);
        config.validate().expect("valid media watchdog timeouts");
    }

    #[test]
    fn diagnostic_debug_omits_config_auth_routing_and_registration_secrets() {
        let mut config = Config::local("local-name-secret", 5060);
        config.local_uri = "sip:local-uri-secret@example.invalid".into();
        config.state_table_path = Some("state-table-secret".into());
        config.credentials = Some(crate::types::Credentials::new(
            "credential-user-secret",
            "credential-password-secret",
        ));
        config.auth = Some(crate::auth::SipClientAuth::bearer_token(
            "bearer-policy-secret",
        ));
        config.pai_uri = Some("sip:pai-secret@example.invalid".into());
        config.outbound_proxy_uri = Some("sip:proxy-secret@example.invalid".into());
        config.sip_instance = Some("instance-secret".into());
        config.contact_uri = Some("sip:contact-secret@example.invalid".into());
        config.tls_cert_path = Some("/cert/path-secret.pem".into());
        config.tls_key_path = Some("/key/path-secret.pem".into());
        config.tls_client_cert_path = Some("/client-cert/path-secret.pem".into());
        config.tls_client_key_path = Some("/client-key/path-secret.pem".into());
        config.tls_extra_ca_path = Some("/ca/path-secret.pem".into());
        config.tls_server_client_auth =
            rvoip_sip_transport::transport::tls::TlsServerClientAuthConfig::required(
                "/client-ca/path-secret.pem",
            );
        config.stun_server = Some("stun-secret.example.invalid".into());
        config.trace_redaction = Some(Arc::new(CanaryTracePolicy("trace-policy-secret")));

        let config_debug = format!("{config:?}");
        for secret in [
            "local-name-secret",
            "local-uri-secret",
            "state-table-secret",
            "credential-user-secret",
            "credential-password-secret",
            "bearer-policy-secret",
            "pai-secret",
            "proxy-secret",
            "instance-secret",
            "contact-secret",
            "path-secret",
            "stun-secret",
            "trace-policy-secret",
        ] {
            assert!(
                !config_debug.contains(secret),
                "Config Debug leaked {secret}"
            );
        }
        assert!(config_debug.contains("credentials_configured: true"));
        assert!(config_debug.contains("auth_configured: true"));
        assert!(config_debug.contains("outbound_proxy_configured: true"));
        assert!(config_debug.contains("tls_key_configured: true"));
        assert!(config_debug.contains("tls_server_client_auth_mode: Required"));

        let retry = OobAuthRetry {
            header_name: "Authorization-secret".into(),
            header_value: "header-value-secret".into(),
            cseq: 7,
            call_id: Some("call-routing-secret".into()),
            from_tag: Some("from-tag-secret".into()),
            nonce: "nonce-secret".into(),
            stale: true,
        };
        let retry_debug = format!("{retry:?}");
        for secret in [
            "Authorization-secret",
            "header-value-secret",
            "call-routing-secret",
            "from-tag-secret",
            "nonce-secret",
        ] {
            assert!(!retry_debug.contains(secret), "retry Debug leaked {secret}");
        }
        assert!(retry_debug.contains("nonce_configured: true"));
        assert!(retry_debug.contains("stale: true"));

        let registration = Registration::new(
            "sip:registrar-secret@example.invalid",
            "registration-user-secret",
            "registration-password-secret",
        )
        .from_uri("sip:from-secret@example.invalid")
        .contact_uri("sip:registration-contact-secret@example.invalid");
        let registration_debug = format!("{registration:?}");
        for secret in [
            "registrar-secret",
            "registration-user-secret",
            "registration-password-secret",
            "from-secret",
            "registration-contact-secret",
        ] {
            assert!(
                !registration_debug.contains(secret),
                "Registration Debug leaked {secret}"
            );
        }
        assert!(registration_debug.contains("password_configured: true"));

        let session_id = crate::state_table::types::SessionId::new();
        let session_secret = session_id.to_string();
        let handle = RegistrationHandle {
            session_id: session_id.clone(),
        };
        assert!(!format!("{handle:?}").contains(&session_secret));
        let info = RegistrationInfo {
            session_id,
            status: RegistrationStatus::Failed,
            registrar: Some("info-registrar-secret".into()),
            contact: Some("info-contact-secret".into()),
            expires_secs: Some(300),
            next_refresh_in: Some(Duration::from_secs(60)),
            retry_count: 2,
            last_failure: Some("failure-detail-secret".into()),
            accepted_expires_secs: Some(300),
            registered_at: Some(Instant::now()),
            next_refresh_at: Some(Instant::now()),
            service_route: Some(vec!["sip:service-route-secret@example.invalid".into()]),
            pub_gruu: Some("pub-gruu-secret".into()),
            temp_gruu: Some("temp-gruu-secret".into()),
            outbound_flow_active: true,
        };
        let info_debug = format!("{info:?}");
        for secret in [
            &session_secret,
            "info-registrar-secret",
            "info-contact-secret",
            "failure-detail-secret",
            "service-route-secret",
            "pub-gruu-secret",
            "temp-gruu-secret",
        ] {
            assert!(
                !info_debug.contains(secret),
                "RegistrationInfo Debug leaked {secret}"
            );
        }
        assert!(info_debug.contains("status: Failed"));
        assert!(info_debug.contains("service_route_count: 1"));
    }
}

#[cfg(all(test, feature = "perf-tests"))]
mod perf_config_tests {
    use super::{dialog_manager_retention_snapshot, Config};
    use rvoip_sip_dialog::manager::core::DialogManagerRetentionCounts;

    #[test]
    fn perf_rss_growth_default_is_ten_mb_per_hour() {
        assert_eq!(Config::DEFAULT_PERF_MAX_RSS_GROWTH_MB_PER_HR, 10.0);
        assert_eq!(
            Config::local("alice", 5060).perf_max_rss_growth_mb_per_hr,
            None
        );
    }

    #[test]
    fn perf_rss_growth_override_validates() {
        let config = Config::local("alice", 5060).with_perf_max_rss_growth_mb_per_hr(2.5);
        assert_eq!(config.perf_max_rss_growth_mb_per_hr, Some(2.5));
        config.validate().expect("valid perf rss threshold");
    }

    #[test]
    fn perf_rss_growth_rejects_invalid_values() {
        for limit in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let config = Config::local("alice", 5060).with_perf_max_rss_growth_mb_per_hr(limit);
            let error = config
                .validate()
                .expect_err("invalid perf rss threshold should fail");
            assert!(
                error.to_string().contains("perf_max_rss_growth_mb_per_hr"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn perf_dialog_snapshot_exposes_invite_failover_retention_counts() {
        let snapshot = dialog_manager_retention_snapshot(DialogManagerRetentionCounts {
            invite_failover_plans: 2,
            active_invite_failover_by_dialog: 3,
            invite_failover_plans_by_dialog: 4,
            invite_failover_attempts: 5,
            invite_failover_attempts_by_dialog: 6,
            invite_failover_plan_reservations: 7,
            invite_failover_attempt_reservations: 11,
            ..DialogManagerRetentionCounts::default()
        });

        assert_eq!(
            snapshot
                .pointer("/invite_failover_plans")
                .and_then(serde_json::Value::as_u64),
            Some(2)
        );
        assert_eq!(
            snapshot
                .pointer("/active_invite_failover_by_dialog")
                .and_then(serde_json::Value::as_u64),
            Some(3)
        );
        assert_eq!(
            snapshot
                .pointer("/invite_failover_plans_by_dialog")
                .and_then(serde_json::Value::as_u64),
            Some(4)
        );
        assert_eq!(
            snapshot
                .pointer("/invite_failover_attempts")
                .and_then(serde_json::Value::as_u64),
            Some(5)
        );
        assert_eq!(
            snapshot
                .pointer("/invite_failover_attempts_by_dialog")
                .and_then(serde_json::Value::as_u64),
            Some(6)
        );
        assert_eq!(
            snapshot
                .pointer("/invite_failover_plan_reservations")
                .and_then(serde_json::Value::as_u64),
            Some(7)
        );
        assert_eq!(
            snapshot
                .pointer("/invite_failover_attempt_reservations")
                .and_then(serde_json::Value::as_u64),
            Some(11)
        );
    }
}

#[cfg(feature = "perf-tests")]
fn dialog_manager_retention_snapshot(
    counts: rvoip_sip_dialog::manager::core::DialogManagerRetentionCounts,
) -> serde_json::Value {
    serde_json::json!({
        "dialogs": counts.dialogs,
        "dialog_lookup": counts.dialog_lookup,
        "early_dialog_lookup": counts.early_dialog_lookup,
        "terminated_bye_lookup": counts.terminated_bye_lookup,
        "terminated_bye_deadlines": counts.terminated_bye_deadlines,
        "transaction_to_dialog": counts.transaction_to_dialog,
        "transaction_dialog_route_hash": counts.transaction_dialog_route_hash,
        "dialog_invite_transactions": counts.dialog_invite_transactions,
        "invite_failover_plans": counts.invite_failover_plans,
        "active_invite_failover_by_dialog": counts.active_invite_failover_by_dialog,
        "invite_failover_plans_by_dialog": counts.invite_failover_plans_by_dialog,
        "invite_failover_attempts": counts.invite_failover_attempts,
        "invite_failover_attempts_by_dialog": counts.invite_failover_attempts_by_dialog,
        "invite_failover_plan_reservations": counts.invite_failover_plan_reservations,
        "invite_failover_attempt_reservations": counts.invite_failover_attempt_reservations,
        "dialog_server_transactions": counts.dialog_server_transactions,
        "pending_response_transaction_by_dialog": counts.pending_response_transaction_by_dialog,
        "session_to_dialog": counts.session_to_dialog,
        "dialog_to_session": counts.dialog_to_session,
        "reliable_provisional_tasks": counts.reliable_provisional_tasks,
        "session_refresh_tasks": counts.session_refresh_tasks,
        "outbound_flows": counts.outbound_flows,
        "outbound_flow_tasks": counts.outbound_flow_tasks,
        "flow_by_destination": counts.flow_by_destination,
        "flow_by_aor": counts.flow_by_aor,
    })
}

fn default_session_event_dispatcher_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 16)
}

/// Lower-level coordinator for SIP sessions, registrations, media, and events.
///
/// `UnifiedCoordinator` is intentionally explicit: most methods take or return
/// a [`SessionId`], and event consumers choose whether to subscribe to all
/// events or filter by session. This makes it suitable for applications that
/// manage more than one call leg at a time.
///
/// Use higher-level wrappers when possible:
///
/// - [`StreamPeer`](crate::api::stream_peer::StreamPeer) for sequential clients
///   and tests.
/// - [`CallbackPeer`](crate::api::callback_peer::CallbackPeer) for reactive
///   servers.
#[allow(dead_code)]
pub struct UnifiedCoordinator {
    /// State machine helpers
    pub(crate) helpers: Arc<StateMachineHelpers>,

    /// Media adapter for audio operations
    media_adapter: Arc<MediaAdapter>,

    /// Dialog adapter for SIP operations
    dialog_adapter: Arc<DialogAdapter>,

    /// Incoming call receiver
    incoming_rx: Arc<RwLock<mpsc::Receiver<IncomingCallInfo>>>,

    /// Global event coordinator — used to publish and subscribe to session API events.
    /// Events are published to the "session_to_app" channel.
    pub(crate) global_coordinator: Arc<GlobalEventCoordinator>,

    /// Single response owner for application-controlled inbound requests.
    session_control_rx: tokio::sync::Mutex<Option<mpsc::Receiver<crate::api::events::Event>>>,

    /// Becomes true only after a public peer surface claims the control queue.
    session_control_claimed: Arc<AtomicBool>,

    /// Configuration
    config: Config,

    /// Shutdown signal — send `true` to stop all background tasks.
    shutdown_tx: tokio::sync::watch::Sender<bool>,

    /// Cancellation-safe singleflight for ordered dependency shutdown.
    shutdown_flights: CoordinatorShutdownFlights,

    /// Per-call lifecycle index for deterministic late waiters.
    lifecycle: LifecycleIndex,

    /// App event publisher that updates lifecycle before global bus delivery.
    app_event_publisher: SessionEventPublisher,

    /// One compact deadline queue shared by every setup/teardown watchdog.
    /// The scheduler task itself retains only a weak coordinator reference.
    setup_teardown_scheduler: Arc<SetupTeardownDeadlineScheduler>,

    /// Strong ownership and one shared deadline queue for application-owned
    /// exact in-dialog responses. This keeps an unanswered request resolvable
    /// even if every application clone is dropped off-runtime.
    pending_exact_responses: Arc<PendingExactResponseRegistry>,

    /// SIP_API_DESIGN_2 Phase A: shared session registry so the four
    /// public surfaces can fetch the parsed inbound `Arc<Request>` when
    /// constructing an `IncomingCall`.
    pub(crate) session_registry: Arc<SessionRegistry>,

    /// Synchronous adapter observers for parsed inbound INVITEs. Observers run
    /// before the corresponding public `IncomingCall` event is published so
    /// an adapter can bind context without a cross-channel ordering race.
    inbound_invite_observers: StdMutex<HashMap<u64, InboundInviteObserver>>,
    next_inbound_invite_observer_id: AtomicU64,

    /// Default UAC Digest credentials adopted from the most recent REGISTER
    /// when the application did not configure [`Config::credentials`] /
    /// [`Config::auth`]. This lets a registered client authenticate challenged
    /// in-account requests (INVITE, re-INVITE, BYE, REFER) out of the box —
    /// most PBXes (Asterisk, FreeSWITCH) challenge those as well as REGISTER.
    /// A plain `std::sync::Mutex` because it is only ever locked briefly and
    /// synchronously (no `.await` held); [`config_credentials`](Self::config_credentials)
    /// reads it.
    registered_credentials: std::sync::Mutex<Option<crate::types::Credentials>>,

    /// Cancellation-safe hangup tasks upgrade this weak self-reference before
    /// detaching. The retained task then owns one strong coordinator reference
    /// through dispatch, final-response confirmation, and exact finalization.
    self_weak: OnceLock<Weak<UnifiedCoordinator>>,
}

async fn run_setup_teardown_deadline_scheduler(
    coordinator: Weak<UnifiedCoordinator>,
    scheduler: Arc<SetupTeardownDeadlineScheduler>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    'scheduler: loop {
        if *shutdown.borrow() {
            break;
        }

        // Register the notification waiter before reading the queue. This
        // closes the insertion-between-peek-and-wait race without retaining a
        // coordinator reference while the scheduler is idle.
        let changed = scheduler.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();

        match scheduler.next_deadline_if_accepting() {
            None => break,
            Some(Some(deadline)) => {
                #[cfg(test)]
                scheduler.record_runner_waiting_for_test(deadline);
                let sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
                tokio::pin!(sleep);
                tokio::select! {
                    _ = &mut changed => {}
                    _ = &mut sleep => {}
                    result = shutdown.changed() => {
                        if result.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
            Some(None) => {
                tokio::select! {
                    _ = &mut changed => {}
                    result = shutdown.changed() => {
                        if result.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
        }

        let Some(due) =
            scheduler.take_due_if_accepting(Instant::now(), SETUP_TEARDOWN_DEADLINE_BATCH)
        else {
            break;
        };
        if due.is_empty() {
            continue;
        }
        let Some(coordinator) = coordinator.upgrade() else {
            for _ in due {
                crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
            }
            break;
        };
        let full_batch = due.len() == SETUP_TEARDOWN_DEADLINE_BATCH;
        let mut due = due.into_iter();
        while let Some(deadline) = due.next() {
            if !coordinator.setup_teardown_deadline_is_current(&deadline) {
                crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
                continue;
            }
            let permit = loop {
                let changed = scheduler.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if scheduler.next_deadline_if_accepting().is_none() {
                    crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
                    for _ in due {
                        crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
                    }
                    break 'scheduler;
                }
                tokio::select! {
                    permit = Arc::clone(&scheduler.fire_slots).acquire_owned() => {
                        match permit {
                            Ok(permit) => break permit,
                            Err(_) => {
                                crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
                                for _ in due {
                                    crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
                                }
                                break 'scheduler;
                            }
                        }
                    }
                    _ = &mut changed => {}
                    result = shutdown.changed() => {
                        if result.is_err() || *shutdown.borrow() {
                            crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
                            for _ in due {
                                crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
                            }
                            break 'scheduler;
                        }
                    }
                }
            };
            if !coordinator.dispatch_setup_teardown_deadline(deadline, permit) {
                crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
                for _ in due {
                    crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
                }
                break 'scheduler;
            }
        }
        drop(coordinator);
        if full_batch {
            tokio::task::yield_now().await;
        }
    }

    scheduler.begin_close();

    // Explicit shutdown and an ungraceful last-owner drop both retire every
    // armed record. This keeps watchdog accounting convergent and releases all
    // generation-qualified identifiers without waiting for their deadlines.
    for _ in scheduler.drain_queued() {
        crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
    }
}

async fn run_exact_response_deadline_scheduler(
    coordinator: Weak<UnifiedCoordinator>,
    registry: Arc<PendingExactResponseRegistry>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            break;
        }
        let changed = registry.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        match registry.next_deadline_if_accepting() {
            None => break,
            Some(Some(deadline)) => {
                let sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
                tokio::pin!(sleep);
                tokio::select! {
                    _ = &mut changed => {}
                    _ = &mut sleep => {}
                    result = shutdown.changed() => {
                        if result.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
            Some(None) => {
                tokio::select! {
                    _ = &mut changed => {}
                    result = shutdown.changed() => {
                        if result.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
        }

        let Some(due) =
            registry.take_due_if_accepting(Instant::now(), EXACT_RESPONSE_DEADLINE_BATCH)
        else {
            break;
        };
        if due.is_empty() {
            continue;
        }
        let Some(coordinator) = coordinator.upgrade() else {
            break;
        };
        let full_batch = due.len() == EXACT_RESPONSE_DEADLINE_BATCH;
        run_bounded_exact_response_batch(
            due,
            EXACT_RESPONSE_DEADLINE_CONCURRENCY,
            |transaction| {
                let coordinator = Arc::clone(&coordinator);
                let registry = Arc::clone(&registry);
                async move {
                    let Some(obligation) = registry.obligation(&transaction) else {
                        return;
                    };
                    let outcome = tokio::time::timeout(
                        EXACT_RESPONSE_SEND_ATTEMPT_TIMEOUT,
                        coordinator.author_pending_exact_response(obligation, 501),
                    )
                    .await;
                    let cause = match outcome {
                        Ok(ManagedExactResponseOutcome::Completed) => return,
                        Ok(ManagedExactResponseOutcome::ZeroWireRetryable) => {
                            ExactResponseRetryCause::ZeroWire
                        }
                        Ok(ManagedExactResponseOutcome::Busy) | Err(_) => {
                            ExactResponseRetryCause::BusyOrTimeout
                        }
                    };
                    let plan = registry.retry_plan(&transaction, cause);
                    if plan.slow_path {
                        tracing::warn!(
                            method = %crate::api::incoming::safe_incoming_method_debug_label(transaction.method()),
                            retry_delay_ms = plan.delay.as_millis(),
                            "Exact response fast retry budget exhausted; retaining the obligation on slow retry"
                        );
                    }
                    registry.reschedule(&transaction, plan.delay);
                }
            },
        )
        .await;
        drop(coordinator);
        if full_batch {
            tokio::task::yield_now().await;
        }
    }
}

async fn run_bounded_exact_response_batch<T, F, Fut>(
    items: Vec<T>,
    concurrency: usize,
    operation: F,
) where
    F: FnMut(T) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    use futures::StreamExt;

    futures::stream::iter(items)
        .for_each_concurrent(concurrency, operation)
        .await;
}

/// Quiesce one exact lifetime, release its lower resources, and remove only
/// the registry/store slot captured by the caller. Every delayed terminal
/// path uses this helper so a reused raw SessionId can never redirect cleanup.
pub(crate) async fn release_exact_local_resources(
    session_store: Arc<SessionStore>,
    helpers: Arc<StateMachineHelpers>,
    dialog_adapter: Arc<DialogAdapter>,
    media_adapter: Arc<MediaAdapter>,
    handle: SessionRegistryHandle,
) -> Result<()> {
    // A racing exact owner may already have completed removal. Treat absence
    // of this generation as idempotent success; never redirect cleanup to a
    // newer lifetime that reused the raw SessionId.
    if session_store
        .get_session_retained_snapshot_exact(&handle)
        .is_err()
    {
        return Ok(());
    }
    let teardown = match session_store.quiesce_session_exact(&handle).await {
        Ok(teardown) => teardown,
        Err(_)
            if session_store
                .get_session_retained_snapshot_exact(&handle)
                .is_err() =>
        {
            return Ok(());
        }
        Err(error) => {
            return Err(SessionError::InternalError(format!(
                "exact session quiesce failed (class=lifecycle): {error}"
            )));
        }
    };
    if let TeardownOutcome::Quarantined { reason, .. } = teardown {
        // `quiesce_session_exact` also joins the retained teardown supervisor.
        // A recoverable deadline can therefore remain as sticky diagnostic
        // evidence after the authority has reached Retired. Let exact removal
        // verify the final phase instead of stranding a reclaimable session.
        tracing::warn!(
            session = %handle.session_id(),
            ?reason,
            "exact teardown reported quarantine; attempting final exact reclamation"
        );
    }

    let dialog_result = dialog_adapter.cleanup_session_exact(&handle).await;
    let media_result = media_adapter.cleanup_session_exact(&handle).await;
    helpers.cleanup_session(handle.session_id()).await;
    if let Err(error) = dialog_result {
        return Err(error);
    }
    if let Err(error) = media_result {
        return Err(error);
    }
    match session_store.remove_quiesced_session_exact(&handle) {
        Ok(()) => Ok(()),
        Err(_)
            if session_store
                .get_session_retained_snapshot_exact(&handle)
                .is_err() =>
        {
            Ok(())
        }
        Err(error) => Err(SessionError::InternalError(format!(
            "exact session removal failed (class=lifecycle): {error}"
        ))),
    }
}

/// Retry one exact terminal release after a cooperative scheduling turn.
///
/// Lower dialog/media cleanup is idempotent and the store slot is retained
/// until both complete, so a transient lower-layer failure remains safely
/// retryable without ever targeting a reused raw session identifier.
pub(crate) async fn release_exact_local_resources_with_retry(
    session_store: Arc<SessionStore>,
    helpers: Arc<StateMachineHelpers>,
    dialog_adapter: Arc<DialogAdapter>,
    media_adapter: Arc<MediaAdapter>,
    handle: SessionRegistryHandle,
) -> Result<()> {
    let first = release_exact_local_resources(
        Arc::clone(&session_store),
        Arc::clone(&helpers),
        Arc::clone(&dialog_adapter),
        Arc::clone(&media_adapter),
        handle.clone(),
    )
    .await;
    if first.is_ok() {
        return first;
    }
    tracing::warn!(
        session = %handle.session_id(),
        "exact terminal release failed; retrying the same retained lifetime"
    );
    tokio::task::yield_now().await;
    release_exact_local_resources(
        session_store,
        helpers,
        dialog_adapter,
        media_adapter,
        handle,
    )
    .await
}

/// Best-effort lower-resource cleanup remaining after authoritative session
/// ownership has already been reclaimed.
pub(crate) struct ForcedLocalSessionCleanup {
    handle: Option<SessionRegistryHandle>,
    helpers: Arc<StateMachineHelpers>,
    dialog_adapter: Arc<DialogAdapter>,
    media_adapter: Arc<MediaAdapter>,
    session_store: Arc<SessionStore>,
}

impl ForcedLocalSessionCleanup {
    /// Finish helper, dialog, and media cleanup without relaying lower errors.
    pub(crate) async fn finish(self) {
        let Some(handle) = self.handle else {
            tracing::debug!("forced local reclaim had no exact session owner");
            return;
        };
        if release_exact_local_resources(
            self.session_store,
            self.helpers,
            self.dialog_adapter,
            self.media_adapter,
            handle,
        )
        .await
        .is_err()
        {
            tracing::debug!("forced exact local reclaim reported incomplete cleanup");
        }
    }
}

impl UnifiedCoordinator {
    pub(crate) fn add_inbound_invite_observer(
        &self,
        observer: InboundInviteObserver,
    ) -> Result<u64> {
        let mut observers = self
            .inbound_invite_observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if observers.len() >= MAX_INBOUND_INVITE_OBSERVERS {
            return Err(SessionError::ConfigurationError(
                "inbound INVITE observer capacity reached".to_string(),
            ));
        }
        let observer_id = self
            .next_inbound_invite_observer_id
            .fetch_add(1, Ordering::Relaxed);
        observers.insert(observer_id, observer);
        Ok(observer_id)
    }

    pub(crate) fn remove_inbound_invite_observer(&self, observer_id: u64) {
        self.inbound_invite_observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&observer_id);
    }

    #[cfg(test)]
    pub(crate) fn inbound_invite_observer_count(&self) -> usize {
        self.inbound_invite_observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub(crate) fn notify_inbound_invite_observers(&self, observation: InboundInviteObservation) {
        let observers = self
            .inbound_invite_observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for observer in observers {
            let observation = observation.clone();
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                observer(observation);
            }))
            .is_err()
            {
                tracing::warn!("inbound INVITE observer panicked; ignoring observer failure");
            }
        }
    }

    /// SIP_API_DESIGN_2 Phase C — read-only access to
    /// [`Config::local_uri`] for builder surfaces that need to
    /// pre-populate the `From` URI when the caller passes `None`.
    /// Kept inherent-impl so the surface adapter doesn't need access
    /// to the private `config` field.
    pub fn config_local_uri(&self) -> String {
        self.config.local_uri.clone()
    }

    /// The `Contact` URI to advertise on REGISTER for `user`: the explicit
    /// [`Config::contact_uri`] when set, otherwise the actual bound/advertised
    /// transport address as `sip:{user}@{host}:{port}`.
    ///
    /// A `Contact`'s purpose is to be the reachable transport address the
    /// registrar routes inbound calls to, so this deliberately uses the bound
    /// address — never the port-less AOR ([`Config::local_uri`]), which would
    /// misroute incoming calls to the default SIP port.
    pub fn config_contact_uri(&self, user: &str) -> String {
        if let Some(contact) = &self.config.contact_uri {
            return contact.clone();
        }
        let host = self
            .config
            .sip_advertised_addr
            .unwrap_or(self.config.bind_addr);
        let scheme = if self.config.local_uri.starts_with("sips:") {
            "sips"
        } else {
            "sip"
        };
        format!("{scheme}:{user}@{host}")
    }

    /// SIP_API_DESIGN_2 §7.1 — read-only access to
    /// [`Config::pai_uri`] for outbound builders that need to resolve
    /// the per-call `P-Asserted-Identity` against
    /// [`PaiOverride::Default`](crate::api::send::PaiOverride::Default).
    pub fn config_pai_uri(&self) -> Option<String> {
        self.config.pai_uri.clone()
    }

    /// Read-only access to the peer-level default Digest credentials so
    /// outbound builders can fall back when the application did not stage
    /// per-call credentials via `with_credentials(..)`.
    ///
    /// Precedence: explicit [`Config::credentials`] first, then credentials
    /// adopted from a prior REGISTER (see `registered_credentials`). The latter
    /// makes a registered client able to place calls without separately
    /// populating `Config.credentials`.
    pub fn config_credentials(&self) -> Option<crate::types::Credentials> {
        self.config.credentials.clone().or_else(|| {
            self.registered_credentials
                .lock()
                .ok()
                .and_then(|slot| slot.clone())
        })
    }

    /// Read-only access to [`Config::auth`] so outbound builders can fall
    /// back to peer-level full-auth configuration.
    pub fn config_auth(&self) -> Option<crate::auth::SipClientAuth> {
        self.config
            .auth
            .clone()
            .or_else(|| self.config.credentials.clone().map(Into::into))
    }

    /// Feature-gated retained-object counts for perf leak investigations.
    ///
    /// This is intentionally not a stable application API. It exists so
    /// release-gate tests can prove that completed call churn is not retaining
    /// per-call state.
    #[cfg(feature = "perf-tests")]
    #[doc(hidden)]
    pub async fn perf_diagnostic_snapshot(&self) -> serde_json::Value {
        let session_stats = self.helpers.state_machine.store.get_stats().await;
        let helper_counts = self.helpers.perf_diagnostic_counts().await;
        let registry_sessions = self.session_registry.session_count().await;
        let cleanup = crate::cleanup_diag::snapshot();
        let admission = crate::admission_diag::snapshot();
        let dialog_diag = rvoip_sip_dialog::diagnostics::snapshot();
        let dialog_core = self.dialog_adapter.dialog_api.dialog_manager().core();
        let transaction_manager = dialog_core.transaction_manager();
        let transaction_counts = transaction_manager.retention_counts();
        let transaction_breakdown = transaction_manager.retention_breakdown();
        let dialog_counts = dialog_core.retention_counts();
        let dialog_breakdown = dialog_core.retention_breakdown();
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        let memory_diagnostics = rvoip_infra_common::memory_diagnostics::snapshot();
        #[cfg(not(feature = "perf-infra-memory-diagnostics"))]
        let memory_diagnostics = serde_json::json!({
            "enabled": false,
            "compiled": false,
            "feature": "perf-infra-memory-diagnostics",
        });
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        let allocator_diagnostics = rvoip_infra_common::memory_diagnostics::allocator_snapshot();
        #[cfg(not(feature = "perf-infra-memory-diagnostics"))]
        let allocator_diagnostics = serde_json::json!({
            "enabled": false,
            "compiled": false,
            "feature": "perf-infra-memory-diagnostics",
        });

        serde_json::json!({
            "config": {
                "sip_transaction_command_channel_capacity": self
                    .config
                    .sip_transaction_command_channel_capacity
                    .unwrap_or(Config::DEFAULT_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY),
                "server_call_capacity": self.config.server_call_capacity,
                "server_retained_lifecycle_capacity": self.config.server_retained_lifecycle_capacity,
                "server_call_admission_limit": self.config.server_call_admission_limit,
                "server_call_admission_soft_limit": self.config.server_call_admission_soft_limit,
                "server_call_admission_pacing_delay_ms": self.config.server_call_admission_pacing_delay_ms,
                "server_overload_retry_after_secs": self.config.server_overload_retry_after_secs,
            },
            "session_store": {
                "total": session_stats.total,
                "idle": session_stats.idle,
                "initiating": session_stats.initiating,
                "ringing": session_stats.ringing,
                "active": session_stats.active,
                "on_hold": session_stats.on_hold,
                "terminating": session_stats.terminating,
                "terminated": session_stats.terminated,
                "failed": session_stats.failed,
                "lifecycle": self.helpers.state_machine.store.perf_lifecycle_counts(),
            },
            "session_registry": {
                "sessions": registry_sessions,
                "lifecycle": self.session_registry.perf_lifecycle_counts(),
            },
            "lifecycle": self.lifecycle.perf_diagnostic_counts(),
            "app_event_publisher": self.app_event_publisher.perf_diagnostic_counts(),
            "global_event_bus": self.global_coordinator.event_bus_diagnostic_snapshot(),
            "state_machine_helpers": helper_counts,
            "transaction_manager": {
                "client_transactions": transaction_counts.client_transactions,
                "server_transactions": transaction_counts.server_transactions,
                "total": transaction_counts.active_transactions_total,
                "terminated_transactions": transaction_counts.terminated_transactions,
                "server_invite_dialog_index": transaction_counts.server_invite_dialog_index,
                "server_invite_dialog_keys_by_tx": transaction_counts.server_invite_dialog_keys_by_tx,
                "invite_2xx_response_cache": transaction_counts.invite_2xx_response_cache,
                "invite_2xx_response_due_queue": transaction_counts.invite_2xx_response_due_queue,
                "transaction_destinations": transaction_counts.transaction_destinations,
                "retired_client_transactions": transaction_manager.retired_client_transaction_count(),
                "event_subscribers": transaction_counts.event_subscribers,
                "subscriber_to_transactions": transaction_counts.subscriber_to_transactions,
                "transaction_to_subscribers": transaction_counts.transaction_to_subscribers,
                "pending_inbound_bytes": transaction_counts.pending_inbound_bytes,
                "pending_inbound_timing": transaction_counts.pending_inbound_timing,
                "breakdown": transaction_breakdown,
            },
            "dialog_manager": dialog_manager_retention_snapshot(dialog_counts),
            "dialog_manager_storage": dialog_breakdown,
            "dialog_adapter": self.dialog_adapter.perf_diagnostic_counts(),
            "media_adapter": self.media_adapter.perf_diagnostic_counts(),
            "memory_diagnostics": memory_diagnostics,
            "allocator_diagnostics": allocator_diagnostics,
            "server_call_admission": admission,
            "sip_dialog_diagnostics": {
                "transaction_runner": {
                    "started": dialog_diag.transaction_runner_started,
                    "exited": dialog_diag.transaction_runner_exited,
                    "active": dialog_diag.transaction_runner_active,
                    "active_max": dialog_diag.transaction_runner_active_max,
                    "destroy_wake_sent": dialog_diag.transaction_runner_destroy_wake_sent,
                    "destroy_wake_failed": dialog_diag.transaction_runner_destroy_wake_failed,
                },
                "transaction_cleanup": {
                    "enqueued": dialog_diag.termination_cleanup_enqueued,
                    "queue_full": dialog_diag.termination_cleanup_queue_full,
                    "in_flight": dialog_diag.termination_cleanup_in_flight,
                    "max_in_flight": dialog_diag.termination_cleanup_max_in_flight,
                    "removed": dialog_diag.termination_cleanup_removed,
                    "poll_attempts": dialog_diag.termination_cleanup_poll_attempts,
                },
                "invite_2xx_cache": {
                    "insert": dialog_diag.invite_2xx_cache_insert,
                    "expired": dialog_diag.invite_2xx_cache_expired,
                    "ack_removed": dialog_diag.invite_2xx_ack_removed,
                    "maintenance_cache_len_max": dialog_diag.invite_2xx_maintenance_cache_len_max,
                    "maintenance_due_queue_len_max": dialog_diag.invite_2xx_maintenance_due_queue_len_max,
                    "maintenance_due": dialog_diag.invite_2xx_maintenance_due,
                    "maintenance_expired": dialog_diag.invite_2xx_maintenance_expired,
                    "maintenance_capped_ticks": dialog_diag.invite_2xx_maintenance_capped_ticks,
                },
                "bye_tombstone": {
                    "hit": dialog_diag.duplicate_bye_tombstone_hit,
                    "miss": dialog_diag.duplicate_bye_tombstone_miss,
                    "table_size_max": dialog_diag.bye_tombstone_table_size_max,
                },
            },
            "cleanup": {
                "enabled": cleanup.enabled,
                "active_total": cleanup.active_total,
                "setup_teardown_watchdog": {
                    "armed": cleanup.setup_teardown_watchdog_armed,
                    "disarmed": cleanup.setup_teardown_watchdog_disarmed,
                    "fired": cleanup.setup_teardown_watchdog_fired,
                    "transition_failed": cleanup.setup_teardown_watchdog_transition_failed,
                    "release_completed": cleanup.setup_teardown_watchdog_release_completed,
                    "release_failed": cleanup.setup_teardown_watchdog_release_failed,
                    "pending_deadlines": self.setup_teardown_scheduler.len(),
                    "fire_in_flight": SETUP_TEARDOWN_TIMEOUT_CONCURRENCY
                        .saturating_sub(self.setup_teardown_scheduler.fire_slots.available_permits()),
                    "fire_concurrency_limit": SETUP_TEARDOWN_TIMEOUT_CONCURRENCY,
                    "deadline_record_bytes": std::mem::size_of::<SetupTeardownDeadline>(),
                },
                "session_event_dispatch": {
                    "saturated": cleanup.session_event_dispatch_saturated,
                    "dropped": cleanup.session_event_dispatch_dropped,
                    "closed": cleanup.session_event_dispatch_closed,
                    "publication_failed": cleanup.session_event_publication_failed,
                    "publication_timed_out": cleanup.session_event_publication_timed_out,
                    "shutdown_timeouts": cleanup.session_event_dispatch_shutdown_timeouts,
                    "aborted_workers": cleanup.session_event_dispatch_aborted_workers,
                },
            },
        })
    }

    /// SIP_API_DESIGN_2 Phase C — internal accessor to the
    /// [`DialogAdapter`] so send/respond builders can route their
    /// dispatch through the same translation layer used by the legacy
    /// flat methods. Crate-private so the builders are the only
    /// external consumers.
    pub(crate) fn dialog_adapter(&self) -> &Arc<DialogAdapter> {
        &self.dialog_adapter
    }

    /// Send out-of-dialog MESSAGE and retry when a configured UAC auth option
    /// can answer a 401/407 challenge.
    pub(crate) async fn send_message_oob_with_optional_auth(
        &self,
        opts: rvoip_sip_dialog::api::unified::MessageRequestOptions,
        auth: Option<SipClientAuth>,
    ) -> Result<Response> {
        let response = self
            .dialog_adapter
            .send_message_oob_with_options(opts.clone())
            .await?;
        let body = if opts.body.is_empty() {
            None
        } else {
            Some(opts.body.as_ref())
        };
        let Some(retry) = self.build_oob_auth_retry_header(
            Method::Message,
            &response,
            &opts.to_uri,
            body,
            auth.as_ref(),
        )?
        else {
            return Ok(response);
        };

        let mut retry_opts = opts.clone();
        retry_opts.cseq = Some(retry.cseq);
        retry_opts.call_id = retry.call_id.clone();
        retry_opts.from_tag = retry.from_tag.clone();
        let retry_response = self
            .dialog_adapter
            .send_message_oob_with_auth(retry_opts, &retry.header_name, retry.header_value.clone())
            .await?;
        self.maybe_retry_oob_stale(
            Method::Message,
            retry_response,
            retry,
            || opts.to_uri.clone(),
            body,
            auth.as_ref(),
            |cseq, call_id, from_tag, header_name, header_value| {
                let mut opts = opts.clone();
                opts.cseq = Some(cseq);
                opts.call_id = call_id;
                opts.from_tag = from_tag;
                async move {
                    self.dialog_adapter
                        .send_message_oob_with_auth(opts, &header_name, header_value)
                        .await
                }
            },
        )
        .await
    }

    /// Send out-of-dialog OPTIONS and retry when a configured UAC auth option
    /// can answer a 401/407 challenge.
    pub(crate) async fn send_options_oob_with_optional_auth(
        &self,
        opts: rvoip_sip_dialog::api::unified::OptionsRequestOptions,
        auth: Option<SipClientAuth>,
    ) -> Result<Response> {
        let response = self
            .dialog_adapter
            .send_options_oob_with_options(opts.clone())
            .await?;
        let Some(retry) = self.build_oob_auth_retry_header(
            Method::Options,
            &response,
            &opts.to_uri,
            None,
            auth.as_ref(),
        )?
        else {
            return Ok(response);
        };

        let mut retry_opts = opts.clone();
        retry_opts.cseq = Some(retry.cseq);
        retry_opts.call_id = retry.call_id.clone();
        retry_opts.from_tag = retry.from_tag.clone();
        let retry_response = self
            .dialog_adapter
            .send_options_oob_with_auth(retry_opts, &retry.header_name, retry.header_value.clone())
            .await?;
        self.maybe_retry_oob_stale(
            Method::Options,
            retry_response,
            retry,
            || opts.to_uri.clone(),
            None,
            auth.as_ref(),
            |cseq, call_id, from_tag, header_name, header_value| {
                let mut opts = opts.clone();
                opts.cseq = Some(cseq);
                opts.call_id = call_id;
                opts.from_tag = from_tag;
                async move {
                    self.dialog_adapter
                        .send_options_oob_with_auth(opts, &header_name, header_value)
                        .await
                }
            },
        )
        .await
    }

    /// Send out-of-dialog SUBSCRIBE and retry when a configured UAC auth option
    /// can answer a 401/407 challenge.
    pub(crate) async fn send_subscribe_oob_with_optional_auth(
        &self,
        target: &str,
        opts: rvoip_sip_dialog::api::unified::SubscribeRequestOptions,
        auth: Option<SipClientAuth>,
    ) -> Result<Response> {
        let response = self
            .dialog_adapter
            .send_subscribe_oob_with_options(target, opts.clone())
            .await?;
        let Some(retry) = self.build_oob_auth_retry_header(
            Method::Subscribe,
            &response,
            target,
            None,
            auth.as_ref(),
        )?
        else {
            return Ok(response);
        };

        let mut retry_opts = opts.clone();
        retry_opts.cseq = Some(retry.cseq);
        retry_opts.call_id = retry.call_id.clone();
        retry_opts.from_tag = retry.from_tag.clone();
        let retry_response = self
            .dialog_adapter
            .send_subscribe_oob_with_auth(
                target,
                retry_opts,
                &retry.header_name,
                retry.header_value.clone(),
            )
            .await?;
        self.maybe_retry_oob_stale(
            Method::Subscribe,
            retry_response,
            retry,
            || target.to_string(),
            None,
            auth.as_ref(),
            |cseq, call_id, from_tag, header_name, header_value| {
                let mut opts = opts.clone();
                opts.cseq = Some(cseq);
                opts.call_id = call_id;
                opts.from_tag = from_tag;
                async move {
                    self.dialog_adapter
                        .send_subscribe_oob_with_auth(target, opts, &header_name, header_value)
                        .await
                }
            },
        )
        .await
    }

    fn build_oob_auth_retry_header(
        &self,
        method: Method,
        response: &Response,
        request_uri: &str,
        body: Option<&[u8]>,
        auth: Option<&SipClientAuth>,
    ) -> Result<Option<OobAuthRetry>> {
        let status = response.status_code();
        if status != 401 && status != 407 {
            return Ok(None);
        }
        let Some(auth) = auth else {
            return Ok(None);
        };

        let (challenge_header_name, auth_header_name) = if status == 407 {
            (HeaderName::ProxyAuthenticate, "Proxy-Authorization")
        } else {
            (HeaderName::WwwAuthenticate, "Authorization")
        };
        let challenge_values = response
            .raw_headers(&challenge_header_name)
            .into_iter()
            .filter_map(|bytes| String::from_utf8(bytes).ok())
            .collect::<Vec<_>>();
        if challenge_values.is_empty() {
            return Err(SessionError::AuthError(format!(
                "{} challenge response missing {}",
                method,
                challenge_header_name.as_str()
            )));
        }
        let challenge_value = challenge_values.join(", ");
        if challenge_value.trim().is_empty() {
            return Err(SessionError::AuthError(format!(
                "{} challenge response missing {}",
                method,
                challenge_header_name.as_str()
            )));
        }
        let transport = self
            .dialog_adapter
            .outbound_transport_context_for_response(response, request_uri);
        let selected = auth
            .authorization_for_challenge_with_transport_context(
                &challenge_value,
                method.as_str(),
                request_uri,
                1,
                body,
                &transport,
            )
            .map_err(|error| {
                crate::errors::redacted_outbound_auth_error(
                    crate::errors::OutboundAuthOperation::Request,
                    error,
                )
            })?;
        let nonce = selected
            .digest_challenge
            .as_ref()
            .map(|challenge| challenge.nonce.clone())
            .unwrap_or_default();
        let cseq = response
            .cseq()
            .map(|cseq| cseq.sequence().saturating_add(1))
            .unwrap_or(2);
        let call_id = response.call_id().map(|call_id| call_id.value());
        let from_tag = response
            .from()
            .and_then(|from| from.tag().map(str::to_string));
        Ok(Some(OobAuthRetry {
            header_name: auth_header_name.to_string(),
            header_value: selected.value,
            cseq,
            call_id,
            from_tag,
            nonce,
            stale: selected.stale,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    async fn maybe_retry_oob_stale<F, Fut>(
        &self,
        method: Method,
        retry_response: Response,
        previous_retry: OobAuthRetry,
        request_uri: impl FnOnce() -> String,
        body: Option<&[u8]>,
        auth: Option<&SipClientAuth>,
        send_retry: F,
    ) -> Result<Response>
    where
        F: FnOnce(u32, Option<String>, Option<String>, String, String) -> Fut,
        Fut: std::future::Future<Output = Result<Response>>,
    {
        if retry_response.status_code() != 401 && retry_response.status_code() != 407 {
            return Ok(retry_response);
        }

        let request_uri = request_uri();
        let Some(stale_retry) = self.build_oob_auth_retry_header(
            method.clone(),
            &retry_response,
            &request_uri,
            body,
            auth,
        )?
        else {
            ensure_retry_not_challenged(method, &retry_response)?;
            return Ok(retry_response);
        };

        if !stale_retry.stale || stale_retry.nonce == previous_retry.nonce {
            ensure_retry_not_challenged(method, &retry_response)?;
            return Ok(retry_response);
        }

        let second_retry_response = send_retry(
            stale_retry.cseq,
            stale_retry.call_id.clone(),
            stale_retry.from_tag.clone(),
            stale_retry.header_name,
            stale_retry.header_value,
        )
        .await?;
        ensure_retry_not_challenged(method, &second_retry_response)?;
        Ok(second_retry_response)
    }

    // ──────────────────────────────────────────────────────────────────
    // SIP_API_DESIGN_2 Phase C — builder entry points.
    //
    // One verb-named entry per outbound method. Each returns a typed
    // builder implementing
    // [`SipRequestOptions`](crate::api::headers::SipRequestOptions),
    // so applications get a uniform `with_header / with_credentials /
    // with_headers_from / strip_header / .send()` shape.
    // ──────────────────────────────────────────────────────────────────

    /// Begin building an outbound INVITE.
    pub fn invite(
        self: &Arc<Self>,
        from: Option<String>,
        to: impl Into<String>,
    ) -> crate::api::send::OutboundCallBuilder {
        crate::api::send::OutboundCallBuilder::new(self.clone(), from, to)
    }

    /// Begin building an outbound BYE.
    pub fn bye(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::send::ByeBuilder {
        crate::api::send::ByeBuilder::new(self.clone(), session.clone())
    }

    /// Begin building an outbound CANCEL.
    pub fn cancel(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::send::CancelBuilder {
        crate::api::send::CancelBuilder::new(self.clone(), session.clone())
    }

    /// Begin building an outbound REFER.
    pub fn refer(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        refer_to: impl Into<String>,
    ) -> crate::api::send::ReferBuilder {
        crate::api::send::ReferBuilder::new(self.clone(), session.clone(), refer_to)
    }

    /// Begin building an outbound NOTIFY.
    pub fn notify(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        event_package: impl Into<String>,
    ) -> crate::api::send::NotifyBuilder {
        crate::api::send::NotifyBuilder::new(self.clone(), session.clone(), event_package)
    }

    /// Begin building an outbound INFO.
    pub fn info(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        content_type: impl Into<String>,
    ) -> crate::api::send::InfoBuilder {
        crate::api::send::InfoBuilder::new(self.clone(), session.clone(), content_type)
    }

    /// Begin building an outbound UPDATE.
    pub fn update(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::send::UpdateBuilder {
        crate::api::send::UpdateBuilder::new(self.clone(), session.clone())
    }

    /// Begin building an outbound re-INVITE.
    pub fn reinvite(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::send::ReInviteBuilder {
        crate::api::send::ReInviteBuilder::new(self.clone(), session.clone())
    }

    /// Begin building an out-of-dialog SUBSCRIBE.
    ///
    /// Canonical SUBSCRIBE-method verb-builder per SIP_API_DESIGN_2.md §3.3.
    /// The legacy state-machine-event observer that previously owned the
    /// bare name was renamed to [`on_session_events`](Self::on_session_events).
    pub fn subscribe(
        self: &Arc<Self>,
        target: impl Into<String>,
        event_package: impl Into<String>,
    ) -> crate::api::send::SubscribeBuilder {
        crate::api::send::SubscribeBuilder::new(self.clone(), target, event_package)
    }

    /// Begin building an out-of-dialog MESSAGE.
    pub fn message(
        self: &Arc<Self>,
        target: impl Into<String>,
    ) -> crate::api::send::MessageBuilder {
        crate::api::send::MessageBuilder::new(self.clone(), target)
    }

    /// Begin building an out-of-dialog OPTIONS.
    pub fn options(
        self: &Arc<Self>,
        target: impl Into<String>,
    ) -> crate::api::send::OptionsBuilder {
        crate::api::send::OptionsBuilder::new(self.clone(), target)
    }

    /// Begin building an accept response for an inbound INVITE.
    pub fn accept(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::respond::AcceptBuilder {
        crate::api::respond::AcceptBuilder::new(self.clone(), session.clone())
    }

    /// Begin building a reject response for an inbound INVITE.
    pub fn reject(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::respond::RejectBuilder {
        crate::api::respond::RejectBuilder::new(self.clone(), session.clone())
    }

    /// Begin building a redirect response for an inbound INVITE.
    pub fn redirect(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::respond::RedirectBuilder {
        crate::api::respond::RedirectBuilder::new(self.clone(), session.clone())
    }

    /// Begin building a UAS-side auth challenge (401 / 407).
    pub fn challenge(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        scheme: crate::api::respond::AuthScheme,
    ) -> crate::api::respond::AuthChallengeBuilder {
        crate::api::respond::AuthChallengeBuilder::new(
            self.clone(),
            session.clone(),
            rvoip_sip_core::types::Method::Invite,
            scheme,
        )
    }

    /// Begin building an outbound REGISTER.
    ///
    /// Canonical REGISTER verb-builder per SIP_API_DESIGN_2.md §3.3. The
    /// legacy 6-arg `register(uri, from, contact, user, pw, exp)` method
    /// was deleted in Phase 12; use this builder entry with
    /// `.with_expires(...)`, `.with_extra_headers(...)`, etc. before
    /// terminating with `.send()`.
    pub fn register(
        self: &Arc<Self>,
        registrar: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> crate::api::send::RegisterBuilder {
        crate::api::send::RegisterBuilder::new(self.clone(), registrar, user, password)
    }

    /// Begin building a generic UAS response (3xx / 4xx / 5xx / 6xx)
    /// for the given session.
    pub fn respond(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        status: u16,
    ) -> crate::errors::Result<crate::api::respond::GenericResponseBuilder> {
        // The coordinator-level `respond` entry is reachable from session
        // state where the inbound INVITE is the only outstanding UAS
        // request; pass `Method::Invite` so the policy classifier
        // matches the UAS context.
        crate::api::respond::GenericResponseBuilder::new(
            self.clone(),
            session.clone(),
            rvoip_sip_core::types::Method::Invite,
            status,
        )
    }

    /// Begin building a reliable 1xx provisional response.
    pub fn send_provisional(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        code: u16,
    ) -> crate::api::respond::ProvisionalBuilder {
        crate::api::respond::ProvisionalBuilder::new(self.clone(), session.clone(), code)
    }
}

#[derive(Clone)]
struct OobAuthRetry {
    header_name: String,
    header_value: String,
    cseq: u32,
    call_id: Option<String>,
    from_tag: Option<String>,
    nonce: String,
    stale: bool,
}

impl std::fmt::Debug for OobAuthRetry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OobAuthRetry")
            .field("header_configured", &!self.header_name.is_empty())
            .field("header_value_configured", &!self.header_value.is_empty())
            .field("cseq", &self.cseq)
            .field("call_id_configured", &self.call_id.is_some())
            .field("from_tag_configured", &self.from_tag.is_some())
            .field("nonce_configured", &!self.nonce.is_empty())
            .field("stale", &self.stale)
            .finish()
    }
}

fn ensure_retry_not_challenged(method: Method, response: &Response) -> Result<()> {
    let status = response.status_code();
    if status == 401 || status == 407 {
        return Err(SessionError::RequestAuthRetryExhausted { method });
    }
    Ok(())
}

impl UnifiedCoordinator {
    /// Create and start a new coordinator.
    ///
    /// This validates [`Config`], initializes dialog and media adapters,
    /// starts the central event handler, and returns a shared coordinator
    /// handle. Background tasks are stopped by calling [`shutdown`](Self::shutdown)
    /// or by dropping all coordinator owners.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// use rvoip_sip::{Config, UnifiedCoordinator};
    ///
    /// let coordinator = UnifiedCoordinator::new(Config::local("alice", 5060)).await?;
    /// coordinator.shutdown_gracefully(None).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(config: Config) -> Result<Arc<Self>> {
        Self::new_with_listener_auth_and_nat(
            config,
            crate::auth::SipListenerAuthPolicy::disabled(),
            SipNatConfig::default(),
        )
        .await
    }

    /// Create a coordinator with explicit SIP/RTP NAT behavior.
    pub async fn new_with_nat(config: Config, nat: SipNatConfig) -> Result<Arc<Self>> {
        Self::new_with_listener_auth_and_nat(
            config,
            crate::auth::SipListenerAuthPolicy::disabled(),
            nat,
        )
        .await
    }

    /// Create a coordinator with listener authentication installed before the
    /// transaction receive loop starts.
    pub async fn new_with_listener_auth(
        config: Config,
        listener_auth_policy: crate::auth::SipListenerAuthPolicy,
    ) -> Result<Arc<Self>> {
        Self::new_with_listener_auth_and_nat(config, listener_auth_policy, SipNatConfig::default())
            .await
    }

    /// Create a coordinator with listener authentication and explicit NAT
    /// behavior installed before any signaling or media task starts.
    pub async fn new_with_listener_auth_and_nat(
        mut config: Config,
        listener_auth_policy: crate::auth::SipListenerAuthPolicy,
        nat: SipNatConfig,
    ) -> Result<Arc<Self>> {
        // Treat an explicitly absent policy as the safe default too. Verbatim
        // tracing requires `trace_passthrough_for_development` or an explicit
        // `PassthroughRedactor`; absence is never a credential-leaking mode.
        if config.trace_redaction.is_none() {
            config.trace_redaction = Some(crate::api::trace_redactor::default_trace_redactor());
        }
        if config
            .trace_redaction
            .as_ref()
            .is_some_and(|redactor| redactor.allows_verbatim_trace())
        {
            // A verbatim trait policy is an explicit development/operator
            // decision. Mirror it into the transport policy so the lower
            // defense-in-depth sanitizer does not silently re-redact the trace.
            config.sip_trace = config.sip_trace.verbatim_for_development();
        }
        config.validate()?;
        nat.validate()?;
        listener_auth_policy.validate()?;
        if listener_auth_policy.has_verified_mtls_peers()
            && config.tls_server_client_auth.mode
                == rvoip_sip_transport::transport::tls::TlsClientAuthMode::Disabled
        {
            return Err(SessionError::ConfigError(
                "SIP listener mTLS fingerprint mappings require transport-level TLS client-certificate verification"
                    .to_string(),
            ));
        }
        rvoip_sip_transport::diagnostics::set_enabled(config.sip_udp_diagnostics);
        rvoip_sip_dialog::diagnostics::set_enabled(config.sip_udp_diagnostics);
        rvoip_sip_dialog::diagnostics::set_transaction_timing_enabled(
            config.sip_transaction_timing_diagnostics,
        );
        rvoip_sip_dialog::diagnostics::set_dialog_timing_enabled(
            config.sip_dialog_timing_diagnostics,
        );
        rvoip_media_core::diagnostics::set_enabled(config.media_setup_diagnostics);
        crate::cleanup_diag::set_enabled(config.cleanup_diagnostics);
        crate::cleanup_diag::set_event_logs_enabled(config.cleanup_diagnostic_events);
        #[cfg(feature = "perf-tests")]
        crate::admission_diag::set_enabled(
            config.cleanup_diagnostics
                || config.sip_transaction_timing_diagnostics
                || config.sip_dialog_timing_diagnostics,
        );
        crate::adapters::media_adapter::set_sdp_diagnostics(
            config.srtp_diagnostics,
            config.media_sdp_diagnostics,
        );
        rvoip_rtp_core::transport::set_udp_diagnostics(
            config.srtp_diagnostics,
            config.rtp_diagnostics,
        );

        let global_event_config = rvoip_infra_common::events::EventCoordinatorConfig::monolithic()
            .with_channel_capacity(config.global_event_channel_capacity);
        let global_coordinator = Arc::new(
            rvoip_infra_common::events::GlobalEventCoordinator::new(global_event_config)
                .await
                .map_err(|e| {
                    SessionError::InternalError(format!(
                        "Failed to create global event coordinator: {}",
                        e
                    ))
                })?,
        );
        let (session_control_tx, session_control_rx) =
            mpsc::channel(config.session_event_dispatcher_channel_capacity);
        let session_control_claimed = Arc::new(AtomicBool::new(false));

        // Create core components
        let authority = match (
            config.server_call_capacity,
            config.server_retained_lifecycle_capacity,
        ) {
            (None, None) => SessionLeaseAuthority::new(),
            (Some(active_capacity), None) => SessionLeaseAuthority::with_capacity(active_capacity),
            (Some(active_capacity), Some(retained_capacity)) => {
                SessionLeaseAuthority::with_capacities(active_capacity, retained_capacity)
                    .map_err(|error| SessionError::ConfigError(error.to_string()))?
            }
            (None, Some(_)) => {
                return Err(SessionError::ConfigError(
                    "server_retained_lifecycle_capacity requires server_call_capacity".to_string(),
                ));
            }
        };
        let registry = Arc::new(SessionRegistry::with_authority(Arc::clone(&authority)));
        let store = Arc::new(SessionStore::with_lifecycle(
            authority,
            Arc::clone(&registry),
            config.server_call_capacity,
        ));

        let sip_trace_owner_id = config
            .sip_trace
            .enabled
            .then(|| format!("sip-trace-{}", uuid::Uuid::new_v4()));

        // Create adapters
        let dialog_api = Self::create_dialog_api(
            &config,
            global_coordinator.clone(),
            sip_trace_owner_id.clone(),
            listener_auth_policy,
        )
        .await?;

        // E4: parse the outbound proxy URI once up-front so a malformed
        // config fails loudly at coordinator boot, not per-call.
        let outbound_proxy_uri = if let Some(s) = config.outbound_proxy_uri.as_ref() {
            use std::str::FromStr;
            match rvoip_sip_core::types::uri::Uri::from_str(s) {
                Ok(u) => Some(u),
                Err(e) => {
                    return Err(crate::errors::SessionError::ConfigurationError(format!(
                        "Config.outbound_proxy_uri ({}) is not a valid SIP URI: {}",
                        s, e
                    )));
                }
            }
        } else {
            None
        };

        // Build RFC 5626 outbound Contact params from config. Require both
        // the outbound flag and a stable instance URN unless validation has
        // already made that mode mandatory.
        let outbound_contact_params = if config.sip_outbound_enabled
            || matches!(
                config.sip_contact_mode,
                SipContactMode::RegisteredFlowRfc5626
            ) {
            if let Some(instance) = config.sip_instance.as_ref() {
                Some(rvoip_sip_core::types::outbound::OutboundContactParams {
                    instance_urn: instance.clone(),
                    reg_id: 1,
                })
            } else {
                tracing::warn!(
                    "Config.sip_outbound_enabled is true but sip_instance is None; \
                     falling back to pre-5626 REGISTER Contact. Provide a stable \
                     urn:uuid:<uuid> in Config.sip_instance to enable RFC 5626."
                );
                None
            }
        } else {
            None
        };

        let symmetric_flow_params = if matches!(
            config.sip_contact_mode,
            SipContactMode::RegisteredFlowSymmetric
        ) {
            Some(rvoip_sip_core::types::outbound::OutboundContactParams {
                instance_urn: config
                    .sip_instance
                    .clone()
                    .unwrap_or_else(|| format!("symmetric:{}", config.local_uri)),
                reg_id: 1,
            })
        } else {
            None
        };

        // Thread the registered-flow keep-alive interval into the
        // DialogManager so REGISTER 2xx responses can spawn CRLFCRLF
        // ping tasks. RFC 5626 mode starts after outbound Contact echo;
        // symmetric mode starts after a successful REGISTER.
        if (outbound_contact_params.is_some() || symmetric_flow_params.is_some())
            && config.outbound_keepalive_interval_secs > 0
        {
            dialog_api
                .dialog_manager()
                .core()
                .set_outbound_keepalive_interval(Some(std::time::Duration::from_secs(
                    config.outbound_keepalive_interval_secs,
                )));
        }

        let dialog_adapter = Arc::new(DialogAdapter::new(
            dialog_api,
            store.clone(),
            global_coordinator.clone(),
            outbound_proxy_uri,
            outbound_contact_params,
            symmetric_flow_params,
            config.registration_auto_refresh,
            config.registration_refresh_jitter_percent,
            config.auto_emit_extra_headers.clone(),
            config.trace_redaction.clone(),
        ));

        let media_controller =
            Self::create_media_controller(&config, global_coordinator.clone(), nat.symmetric_rtp)
                .await?;
        let mut media_adapter_inner = MediaAdapter::new(
            media_controller,
            store.clone(),
            config.local_ip,
            config.media_port_start,
            config.media_port_end,
        );
        media_adapter_inner.set_media_mode(config.media_mode);
        // Apply RFC 4568 SDES-SRTP policy from Config (Step 2B.1).
        media_adapter_inner.set_srtp_policy(
            config.offer_srtp,
            config.srtp_required,
            config.srtp_offered_suites.clone(),
        );
        // Sprint 3 C1 — propagate Comfort Noise opt-in.
        media_adapter_inner.set_comfort_noise(config.comfort_noise_enabled);
        // Sprint 3.5 — propagate strict codec matching policy.
        media_adapter_inner.set_strict_codec_matching(config.strict_codec_matching);
        // NEXT_STEPS C2 — propagate the configured offered codec list.
        media_adapter_inner.set_offered_codecs(config.offered_codecs.clone());
        media_adapter_inner.set_g729_annex_b(config.g729_annex_b);
        let media_adapter = Arc::new(media_adapter_inner);

        // Sprint 3 A6 — resolve the public RTP address. Static
        // override wins over STUN; STUN failure is soft (warn + use
        // local IP). Probe runs once, here, before any session is
        // created.
        let pending_stun_probe = if let Some(static_addr) = config.media_public_addr {
            if config.stun_server.is_some() {
                tracing::warn!(
                    "Both Config::media_public_addr and Config::stun_server are set; \
                     using the static override and skipping the STUN probe"
                );
            }
            tracing::info!(
                "RTP public addr: {} (static override from Config::media_public_addr)",
                static_addr
            );
            media_adapter.set_public_rtp_addr(Some(static_addr));
            None
        } else {
            config.stun_server.clone()
        };
        // RFC 4733 DTMF bridge: adapter publishes `Event::DtmfReceived`
        // onto the API bus whenever media-core signals a DTMF event.
        media_adapter
            .set_global_coordinator(global_coordinator.clone())
            .await;

        // Load state table based on config
        let state_table = Arc::new(crate::state_table::load_state_table_with_config(
            config.state_table_path.as_deref(),
        ));

        let (state_event_tx, state_event_rx) = mpsc::channel::<
            crate::state_machine::executor::SessionEvent,
        >(config.state_event_channel_capacity);

        let state_machine = Arc::new(StateMachine::new_with_custom_table(
            state_table,
            store.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
            state_event_tx,
            config.auto_180_ringing,
        ));

        // Wire the state machine into the dialog adapter (for REGISTER
        // response handling). The adapter holds an `Arc<OnceLock<_>>`
        // internally so this post-construction init is sound without
        // `unsafe`.
        let _ = dialog_adapter.init_state_machine(state_machine.clone());

        // Create helpers
        let helpers = Arc::new(StateMachineHelpers::new(state_machine.clone()));

        // Create incoming call channel
        let (incoming_tx, incoming_rx) = mpsc::channel(config.incoming_call_channel_capacity);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let setup_teardown_shutdown_rx = shutdown_rx.clone();
        let exact_response_shutdown_rx = shutdown_rx.clone();
        let lifecycle = config
            .server_call_capacity
            .map(LifecycleIndex::with_capacity)
            .unwrap_or_default();
        let app_event_publisher = SessionEventPublisher::with_dispatcher(
            global_coordinator.clone(),
            lifecycle.clone(),
            config.session_event_dispatcher_workers,
            config.session_event_dispatcher_channel_capacity,
        )
        .with_control_sink(session_control_tx, Arc::clone(&session_control_claimed));
        media_adapter
            .set_app_event_publisher(app_event_publisher.clone())
            .await;
        let fast_auto_accept_incoming_calls = config.fast_auto_accept_incoming_calls;
        let fast_auto_accept_queue_capacity = config.incoming_call_channel_capacity;
        let server_call_admission_limit = config.server_call_admission_limit;
        let server_call_admission_soft_limit = config.server_call_admission_soft_limit;
        let server_call_admission_pacing_delay_ms = config.server_call_admission_pacing_delay_ms;
        let server_overload_retry_after_secs = config.server_overload_retry_after_secs;

        let coordinator = Arc::new(Self {
            helpers,
            media_adapter: media_adapter.clone(),
            dialog_adapter: dialog_adapter.clone(),
            incoming_rx: Arc::new(RwLock::new(incoming_rx)),
            global_coordinator: global_coordinator.clone(),
            session_control_rx: tokio::sync::Mutex::new(Some(session_control_rx)),
            session_control_claimed,
            config,
            shutdown_tx,
            shutdown_flights: CoordinatorShutdownFlights::default(),
            lifecycle: lifecycle.clone(),
            app_event_publisher: app_event_publisher.clone(),
            setup_teardown_scheduler: Arc::new(SetupTeardownDeadlineScheduler::default()),
            pending_exact_responses: Arc::new(PendingExactResponseRegistry::default()),
            session_registry: registry.clone(),
            inbound_invite_observers: StdMutex::new(HashMap::new()),
            next_inbound_invite_observer_id: AtomicU64::new(1),
            registered_credentials: std::sync::Mutex::new(None),
            self_weak: OnceLock::new(),
        });
        let _ = coordinator.self_weak.set(Arc::downgrade(&coordinator));
        let mut construction_guard = CoordinatorConstructionGuard::new(Arc::clone(&coordinator));
        let setup_teardown_scheduler = Arc::clone(&coordinator.setup_teardown_scheduler);
        let setup_teardown_tasks = Arc::clone(&setup_teardown_scheduler.tasks);
        let scheduler_started = setup_teardown_tasks.spawn(run_setup_teardown_deadline_scheduler(
            Arc::downgrade(&coordinator),
            setup_teardown_scheduler,
            setup_teardown_shutdown_rx,
        ));
        debug_assert!(scheduler_started);
        let exact_response_runner_started =
            setup_teardown_tasks.spawn(run_exact_response_deadline_scheduler(
                Arc::downgrade(&coordinator),
                Arc::clone(&coordinator.pending_exact_responses),
                exact_response_shutdown_rx,
            ));
        debug_assert!(exact_response_runner_started);
        if let Some(stun_target) = pending_stun_probe {
            // Keep boot nonblocking while making constructor cancellation and
            // graceful shutdown join the probe before media dependencies drop.
            let adapter_for_probe = media_adapter.clone();
            let probe_started =
                coordinator
                    .setup_teardown_scheduler
                    .spawn_lifecycle_task(async move {
                        if let Err(e) = run_stun_probe(adapter_for_probe, &stun_target).await {
                            tracing::warn!(
                                "STUN probe failed against '{}': {} — falling back to local IP",
                                stun_target,
                                e
                            );
                        }
                    });
            debug_assert!(probe_started);
        }

        // Start the dialog adapter. The scheduler runner is already retained;
        // join it explicitly on constructor failure rather than relying on a
        // later last-owner drop to wake and detach it.
        if let Err(start_error) = dialog_adapter.start().await {
            if coordinator.cleanup_failed_construction().await {
                construction_guard.disarm();
            }
            return Err(start_error);
        }

        // Create and start the centralized event handler.
        // Events are published to the global coordinator's "session_to_app" channel.
        let event_handler =
            crate::adapters::SessionCrossCrateEventHandler::with_event_broadcast_and_state_machine_events(
                state_machine.clone(),
                global_coordinator.clone(),
                dialog_adapter.clone(),
                media_adapter.clone(),
                registry.clone(),
                incoming_tx,
                state_event_rx,
                app_event_publisher.clone(),
                sip_trace_owner_id,
            )
            .with_fast_auto_accept_incoming_calls(
                fast_auto_accept_incoming_calls,
                fast_auto_accept_queue_capacity,
            )
            .with_server_call_admission(
                server_call_admission_limit,
                server_call_admission_soft_limit,
                server_call_admission_pacing_delay_ms,
                server_overload_retry_after_secs,
            );

        // SIP_API_DESIGN_2 Phase D — give the handler a weak handle
        // back to the coordinator so the bus-path `IncomingRegister`
        // branch can build a response-capable wrapper. Weak avoids
        // the circular ownership loop.
        event_handler.set_coordinator(&coordinator);

        // Start the event handler (sets up channels and subscriptions). Keep
        // the same bottom-up shutdown order if construction fails after the
        // dialog adapter and scheduler have started.
        if let Err(start_error) = event_handler.start(shutdown_rx).await {
            if coordinator.cleanup_failed_construction().await {
                construction_guard.disarm();
            }
            return Err(start_error);
        }

        construction_guard.disarm();
        Ok(coordinator)
    }

    pub(crate) fn fast_auto_accept_incoming_calls(&self) -> bool {
        self.config.fast_auto_accept_incoming_calls
    }

    pub(crate) async fn claim_session_control_events(
        &self,
    ) -> Result<mpsc::Receiver<crate::api::events::Event>> {
        let mut receiver = self.session_control_rx.lock().await;
        let receiver = receiver.take().ok_or_else(|| {
            SessionError::InvalidInput(
                "application control-event receiver already has an owner".to_string(),
            )
        })?;
        self.session_control_claimed.store(true, Ordering::Release);
        Ok(receiver)
    }

    pub(crate) fn register_exact_response_obligation(
        &self,
        obligation: Arc<crate::api::incoming::ExactResponseObligation>,
    ) -> ExactResponseRegistration {
        self.pending_exact_responses.register(obligation)
    }

    pub(crate) fn complete_exact_response_obligation(
        &self,
        transaction: &rvoip_sip_dialog::transaction::TransactionKey,
    ) {
        self.pending_exact_responses.remove(transaction);
    }

    async fn author_pending_exact_response(
        &self,
        obligation: Arc<crate::api::incoming::ExactResponseObligation>,
        status: u16,
    ) -> ManagedExactResponseOutcome {
        let Ok(claim) = obligation.claim() else {
            return ManagedExactResponseOutcome::Busy;
        };
        let transaction = obligation.transaction().clone();
        let result = self
            .dialog_adapter()
            .send_response_for_transaction_classified(
                obligation.call_id(),
                &transaction,
                status,
                None,
            )
            .await;
        match result {
            Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal)
            | Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal) => {
                claim.complete();
                ManagedExactResponseOutcome::Completed
            }
            Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) => {
                claim.release_after_failure();
                ManagedExactResponseOutcome::ZeroWireRetryable
            }
            Err(error)
                if error.disposition
                    == rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable =>
            {
                tracing::warn!(
                    method = %crate::api::incoming::safe_incoming_method_debug_label(transaction.method()),
                    status_code = status,
                    "Managed exact final response failed before transport write: {}",
                    error.source
                );
                claim.release_after_failure();
                ManagedExactResponseOutcome::ZeroWireRetryable
            }
            Err(error) => {
                tracing::warn!(
                    method = %crate::api::incoming::safe_incoming_method_debug_label(transaction.method()),
                    status_code = status,
                    "Managed exact final response became wire-unknown and will not be retried: {}",
                    error.source
                );
                claim.complete();
                ManagedExactResponseOutcome::Completed
            }
        }
    }

    async fn drain_exact_responses_for_shutdown(&self) -> Result<()> {
        self.pending_exact_responses.begin_close();
        let deadline = Instant::now() + EXACT_RESPONSE_SHUTDOWN_DRAIN_TIMEOUT;
        loop {
            let pending = self.pending_exact_responses.snapshot();
            if pending.is_empty() {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(SessionError::InternalError(format!(
                    "exact response shutdown drain timed out with {} pending obligations",
                    self.pending_exact_responses.entries.len()
                )));
            }
            let attempt_timeout = remaining.min(EXACT_RESPONSE_SEND_ATTEMPT_TIMEOUT);
            let batch = run_bounded_exact_response_batch(
                pending,
                EXACT_RESPONSE_DEADLINE_CONCURRENCY,
                |obligation| async move {
                    let _ = tokio::time::timeout(
                        attempt_timeout,
                        self.author_pending_exact_response(obligation, 503),
                    )
                    .await;
                },
            );
            let _ = tokio::time::timeout(remaining, batch).await;
            if self.pending_exact_responses.entries.is_empty() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(SessionError::InternalError(format!(
                    "exact response shutdown drain timed out with {} pending obligations",
                    self.pending_exact_responses.entries.len()
                )));
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    // ===== Shutdown =====

    async fn cleanup_failed_construction(&self) -> bool {
        if let Err(refresh_error) = self
            .dialog_adapter
            .abort_all_registration_refreshes_and_wait()
            .await
        {
            tracing::warn!(
                "Registration refresh tasks failed to join after coordinator construction failure; retaining dependencies: {}",
                refresh_error
            );
            return false;
        }
        self.pending_exact_responses.begin_close();
        let _ = self.shutdown_tx.send(true);
        if let Err(drain_error) = self.drain_exact_responses_for_shutdown().await {
            tracing::warn!(
                "Exact response obligations failed to drain after coordinator construction failure; retaining dependencies: {}",
                drain_error
            );
            return false;
        }
        if let Err(drain_error) = self
            .setup_teardown_scheduler
            .close_and_wait(SETUP_TEARDOWN_SCHEDULER_DRAIN_TIMEOUT)
            .await
        {
            tracing::warn!(
                "Setup/teardown scheduler failed to join after coordinator construction failure; retaining dependencies: {}",
                drain_error
            );
            return false;
        }
        let stop_succeeded = match self.dialog_adapter.stop().await {
            Ok(()) => true,
            Err(stop_error) => {
                tracing::warn!(
                    "Dialog adapter failed to stop after coordinator construction failure: {}",
                    stop_error
                );
                false
            }
        };
        if stop_succeeded {
            self.pending_exact_responses.clear();
            self.app_event_publisher.shutdown().await;
        }
        stop_succeeded
    }

    /// Shut down this coordinator and all its background tasks.
    ///
    /// This is a non-blocking best-effort shutdown. When
    /// [`Config::unregister_on_shutdown_timeout_secs`] is non-zero, active
    /// registrations are asked to unregister before the shutdown signal is
    /// sent. Use [`shutdown_gracefully`](Self::shutdown_gracefully) when the
    /// caller needs deterministic unregister completion.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) {
    /// coordinator.shutdown();
    /// # }
    /// ```
    pub fn shutdown(&self) {
        let timeout = Duration::from_secs(self.config.unregister_on_shutdown_timeout_secs);
        let _ = self.begin_shutdown_attempt(timeout);
    }

    /// Gracefully unregister active registrations, then stop background tasks.
    ///
    /// The timeout applies per registration. Pass `None` to use
    /// [`Config::unregister_on_shutdown_timeout_secs`]. A zero timeout skips
    /// unregister and behaves like an immediate shutdown.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// coordinator.shutdown_gracefully(Some(std::time::Duration::from_secs(2))).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn shutdown_gracefully(&self, timeout: Option<Duration>) -> Result<()> {
        let timeout = timeout.unwrap_or_else(|| {
            Duration::from_secs(self.config.unregister_on_shutdown_timeout_secs)
        });
        self.begin_shutdown_attempt(timeout)
            .wait()
            .await
            .into_result()
    }

    fn begin_shutdown_attempt(&self, timeout: Duration) -> Arc<CoordinatorShutdownAttempt> {
        let (attempt, should_start) = self.shutdown_flights.begin();
        if !should_start {
            return attempt;
        }

        let Some(coordinator) = self.self_weak.get().and_then(Weak::upgrade) else {
            attempt.finish(CoordinatorShutdownOutcome::DriverPanicked);
            return attempt;
        };
        let driver_attempt = Arc::clone(&attempt);
        tokio::spawn(async move {
            let outcome =
                match std::panic::AssertUnwindSafe(coordinator.run_shutdown_sequence(timeout))
                    .catch_unwind()
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(_) => {
                        tracing::error!("Coordinator shutdown driver panicked");
                        CoordinatorShutdownOutcome::DriverPanicked
                    }
                };
            driver_attempt.finish(outcome);
        });
        attempt
    }

    async fn run_shutdown_sequence(&self, timeout: Duration) -> CoordinatorShutdownOutcome {
        if !timeout.is_zero() {
            self.unregister_registered_sessions(timeout).await;
        }
        if let Err(error) = self
            .dialog_adapter
            .abort_all_registration_refreshes_and_wait()
            .await
        {
            tracing::warn!(
                "Registration refresh tasks failed to drain during shutdown; retaining SIP dependencies for retry: {}",
                error
            );
            return CoordinatorShutdownOutcome::RegistrationRefreshDrainFailed;
        }
        // Stop admitting new application-control work while dialog/transport
        // resources are still available. The strong registry owns queued or
        // retained IncomingRequest values and authors an exact 503 for each
        // unclaimed obligation before the shared response supervisor closes.
        self.pending_exact_responses.begin_close();
        let _ = self.shutdown_tx.send(true);
        if let Err(error) = self.drain_exact_responses_for_shutdown().await {
            tracing::warn!(
                "Exact response obligations failed to drain during shutdown; retaining SIP dependencies for retry: {}",
                error
            );
            return CoordinatorShutdownOutcome::ExactResponseDrainFailed;
        }
        if let Err(error) = self
            .setup_teardown_scheduler
            .close_and_wait(SETUP_TEARDOWN_SCHEDULER_DRAIN_TIMEOUT)
            .await
        {
            tracing::warn!(
                "Setup/teardown scheduler failed to drain during shutdown; retaining SIP dependencies for retry: {}",
                error
            );
            return CoordinatorShutdownOutcome::SchedulerDrainFailed;
        }
        if let Err(error) = self.dialog_adapter.stop().await {
            tracing::warn!(
                "Dialog adapter stop failed during shutdown; retaining event routes for retry: {}",
                error
            );
            return CoordinatorShutdownOutcome::DialogStopFailed;
        }
        self.pending_exact_responses.clear();
        self.app_event_publisher.shutdown().await;
        CoordinatorShutdownOutcome::Succeeded
    }

    async fn unregister_registered_sessions(&self, timeout: Duration) {
        let sessions = self.helpers.state_machine.store.get_all_sessions().await;
        for session in sessions {
            if !session.is_registered {
                continue;
            }
            let handle = RegistrationHandle {
                session_id: session.session_id.clone(),
            };
            if let Err(e) = self.unregister_and_wait(&handle, Some(timeout)).await {
                tracing::warn!(
                    "Graceful shutdown unregister failed for session {}: {}",
                    session.session_id,
                    e
                );
            }
        }
    }

    /// Return a cloneable handle that can signal
    /// [`shutdown`](Self::shutdown) from another task. Mirrors
    /// [`CallbackPeer::shutdown_handle`].
    ///
    /// [`CallbackPeer::shutdown_handle`]: crate::api::callback_peer::CallbackPeer::shutdown_handle
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) {
    /// let stop = coordinator.shutdown_handle();
    /// tokio::spawn(async move {
    ///     stop.shutdown();
    /// });
    /// # }
    /// ```
    pub fn shutdown_handle(&self) -> crate::api::callback_peer::ShutdownHandle {
        crate::api::callback_peer::ShutdownHandle::from_coordinator(
            self.self_weak.get().cloned().unwrap_or_else(Weak::new),
        )
    }

    // ===== Event Subscription =====

    /// Subscribe to the raw cross-crate session API event stream.
    ///
    /// Returns an independent `mpsc::Receiver` for events published by this
    /// coordinator on the internal `"session_to_app"` channel. Most
    /// application code should prefer [`events`](Self::events), which wraps
    /// this raw receiver and yields typed [`Event`](crate::api::events::Event)
    /// values.
    ///
    /// Use this method only when building a custom peer type or diagnostic
    /// tool that needs access to the raw event envelope.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// let mut raw_events = coordinator.subscribe_events().await?;
    /// tokio::spawn(async move {
    ///     while let Some(_event) = raw_events.recv().await {
    ///         // Downcast to SessionApiCrossCrateEvent for diagnostics.
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_events(
        &self,
    ) -> crate::errors::Result<
        tokio::sync::mpsc::Receiver<
            std::sync::Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>,
        >,
    > {
        self.global_coordinator
            .subscribe(crate::adapters::SESSION_TO_APP_CHANNEL)
            .await
            .map_err(|e| {
                crate::errors::SessionError::InternalError(format!(
                    "Failed to subscribe to session events: {}",
                    e
                ))
            })
    }

    /// Return a typed, unfiltered [`EventReceiver`](crate::api::stream_peer::EventReceiver) that yields
    /// [`crate::api::events::Event`] values across all sessions and
    /// registration lifecycles owned by this coordinator.
    ///
    /// Use when a single consumer needs every session API event, for example
    /// a b2bua coordinator, activity log, or registration monitor. For
    /// per-leg call logic prefer [`events_for_session`][Self::events_for_session].
    ///
    /// The returned receiver already handles the downcast from the raw
    /// cross-crate broadcast and exposes filtering helpers like
    /// [`EventReceiver::next_dtmf`](crate::api::stream_peer::EventReceiver::next_dtmf),
    /// [`EventReceiver::next_incoming`](crate::api::stream_peer::EventReceiver::next_incoming),
    /// and
    /// [`EventReceiver::next_transfer`](crate::api::stream_peer::EventReceiver::next_transfer).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// use rvoip_sip::Event;
    ///
    /// let mut events = coordinator.events().await?;
    /// if let Some(Event::RegistrationSuccess { registrar, .. }) = events.next().await {
    ///     println!("registered with {registrar}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn events(&self) -> Result<crate::api::stream_peer::EventReceiver> {
        let rx = self.subscribe_events().await?;
        Ok(crate::api::stream_peer::EventReceiver::new(rx))
    }

    /// Claim the coordinator's single response-capable control stream and
    /// merge it with the ordinary public observations.
    ///
    /// Only one control owner may exist. Response-bearing inbound requests
    /// such as SIP INFO are yielded once through the private stream; their
    /// public observational copies are sanitized and suppressed by this
    /// receiver. Further calls return an error. Libraries that only monitor
    /// calls should continue to use [`events`](Self::events).
    pub async fn events_with_control(
        self: &Arc<Self>,
    ) -> Result<crate::api::stream_peer::EventReceiver> {
        let observations = self.subscribe_events().await?;
        let control = self.claim_session_control_events().await?;
        Ok(crate::api::stream_peer::EventReceiver::with_control(
            observations,
            control,
            Arc::clone(self),
        ))
    }

    /// Return an [`EventReceiver`](crate::api::stream_peer::EventReceiver) that only yields events whose
    /// `call_id` matches `id`. Per-session filtering happens in the
    /// receiver's `next()` loop.
    ///
    /// Registration lifecycle events do not carry a call id, so they are only
    /// visible on [`events`](Self::events), not on per-session receivers.
    ///
    /// Intended for b2bua-style consumers that need to watch both legs of
    /// a bridged call independently:
    ///
    /// ```no_run
    /// # use rvoip_sip::{Event, SessionId, UnifiedCoordinator};
    /// # async fn example(coord: &UnifiedCoordinator, inbound: &SessionId, outbound: &SessionId) {
    /// let mut inbound_events = coord.events_for_session(inbound).await.unwrap();
    /// let mut outbound_events = coord.events_for_session(outbound).await.unwrap();
    /// tokio::select! {
    ///     Some(Event::CallEnded { .. }) = inbound_events.next() => {
    ///         // inbound leg ended — tear down the outbound leg
    ///     }
    ///     Some(Event::CallEnded { .. }) = outbound_events.next() => {
    ///         // outbound leg ended — tear down the inbound leg
    ///     }
    /// }
    /// # }
    /// ```
    ///
    /// **Caller contract:** open the receiver *before* any event of
    /// interest fires. Events are lost if no subscriber is attached at
    /// publish time. For incoming calls the safe pattern is:
    /// 1. Wait for an `IncomingCall` event on the unfiltered
    ///    [`events()`][Self::events] receiver.
    /// 2. Open `events_for_session(&id)` with the new `SessionId`.
    /// 3. Call `accept_call_with_sdp()` (post-acceptance events then
    ///    reach the filtered receiver).
    ///
    /// # Examples
    ///
    /// See the b2bua-style event split above for a complete `tokio::select!`
    /// example.
    pub async fn events_for_session(
        &self,
        id: &SessionId,
    ) -> Result<crate::api::stream_peer::EventReceiver> {
        let rx = self.subscribe_events().await?;
        let mut receiver = crate::api::stream_peer::EventReceiver::filtered(rx, id.clone());

        // Race repair: SESSION_TO_APP_CHANNEL is broadcast — a subscriber
        // added here cannot observe events that fired before this call
        // returned. On a fast loopback, `invite → 200 OK → CallAnswered`
        // can complete in well under a millisecond, so callers that follow
        // the documented `invite().send() → events_for_session → wait for
        // CallAnswered` pattern would otherwise deadlock. Inspect the
        // session's *current* state and synthesize the events the caller
        // would have observed had they been subscribed earlier.
        if let Ok(state) = self.helpers.get_state(id).await {
            use crate::types::CallState;
            match state {
                CallState::Active
                | CallState::Bridged
                | CallState::OnHold
                | CallState::HoldPending
                | CallState::EarlyMedia
                | CallState::Resuming
                | CallState::Muted => {
                    receiver.prime(crate::api::events::Event::CallAnswered {
                        call_id: id.clone(),
                        sdp: None,
                    });
                }
                CallState::Failed(reason) => {
                    receiver.prime(crate::api::events::Event::CallFailed {
                        call_id: id.clone(),
                        reason: reason.to_string(),
                        status_code: 500,
                    });
                }
                CallState::Terminated | CallState::Cancelled => {
                    receiver.prime(crate::api::events::Event::CallEnded {
                        call_id: id.clone(),
                        reason: format!("session in state {state:?}"),
                    });
                }
                _ => {}
            }
        }

        Ok(receiver)
    }

    pub(crate) async fn lifecycle_snapshot(&self, id: &SessionId) -> CallLifecycleSnapshot {
        let (state, media_security) = self
            .helpers
            .state_machine
            .store
            .with_session(id, |session| {
                (
                    Some(session.call_state.clone()),
                    session.media_security.clone(),
                )
            })
            .unwrap_or((None, None));
        let mut snapshot = self.lifecycle.snapshot(id, state);
        if snapshot.media_security.is_none() {
            snapshot.media_security = media_security;
        }
        snapshot
    }

    pub(crate) fn lifecycle_watcher(&self, id: &SessionId) -> tokio::sync::watch::Receiver<u64> {
        self.lifecycle.watcher(id)
    }

    #[doc(hidden)]
    pub async fn publish_app_event_for_test(&self, event: crate::api::events::Event) -> Result<()> {
        self.app_event_publisher.publish_now(event).await
    }

    // ===== Simple Call Operations =====

    /// Spawn an outbound leg linked to a transferor session for RFC 3515
    /// §2.4.5 progress reporting. The new leg's `SessionState` carries
    /// `transferor_session_id = Some(..)` before the state machine
    /// dispatches `MakeCall`, so every subsequent `Dialog180Ringing` /
    /// `Dialog200OK` / failure fires a progress NOTIFY back on the
    /// transferor's REFER subscription. This is the b2bua wrapper crate's
    /// primary REFER-forwarding entry point.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, transferor: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let new_leg = coordinator.make_transfer_leg(
    ///     "sip:service@127.0.0.1:5060",
    ///     "sip:target@example.com",
    ///     &transferor,
    /// ).await?;
    /// # let _ = new_leg;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn make_transfer_leg(
        &self,
        from: &str,
        to: &str,
        transferor_session_id: &SessionId,
    ) -> Result<SessionId> {
        self.helpers
            .make_transfer_leg(from, to, transferor_session_id)
            .await
    }

    /// Retroactively link an existing session as a transfer leg of
    /// `transferor_session_id`. Prefer [`make_transfer_leg`](Self::make_transfer_leg) — this
    /// lower-level primitive accepts a race window in which dialog
    /// events fired before the linkage is set silently drop their
    /// corresponding progress NOTIFY.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, leg: rvoip_sip::SessionId, transferor: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.set_transferor_session(&leg, &transferor).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_transferor_session(
        &self,
        leg_session_id: &SessionId,
        transferor_session_id: &SessionId,
    ) -> Result<()> {
        self.helpers
            .set_transferor_session(leg_session_id, transferor_session_id)
            .await
    }

    /// Accept an incoming call.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, incoming: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.accept_call(&incoming).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        let result = self.helpers.accept_call(session_id).await;
        self.schedule_setup_teardown_timeout_if_current(
            session_id,
            SetupTeardownWatchdogKind::AcceptedCall,
        )
        .await;
        result
    }

    /// Accept an incoming call with a caller-supplied SDP answer. Bypasses
    /// local media negotiation — intended for b2bua flows where the answer
    /// body comes from the outbound leg's 200 OK. See
    /// [`StateMachineHelpers::accept_call_with_sdp`] for the mechanism.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, incoming: rvoip_sip::SessionId, answer_sdp: String) -> rvoip_sip::Result<()> {
    /// coordinator.accept_call_with_sdp(&incoming, answer_sdp).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept_call_with_sdp(&self, session_id: &SessionId, sdp: String) -> Result<()> {
        let result = self.helpers.accept_call_with_sdp(session_id, sdp).await;
        self.schedule_setup_teardown_timeout_if_current(
            session_id,
            SetupTeardownWatchdogKind::AcceptedCallWithSdp,
        )
        .await;
        result
    }

    /// Hang up or cancel a call.
    ///
    /// Established calls send BYE. Ringing or early-media outbound calls send
    /// CANCEL and do not publish `CallCancelled` until the INVITE reaches a
    /// terminal outcome. If the outbound INVITE has not received a provisional
    /// response yet, cancel intent is recorded and CANCEL is sent only if it
    /// later becomes legal; a fast 200 OK is ACKed and immediately BYE-cleaned.
    /// Use [`SessionHandle::hangup_and_wait`](crate::api::handle::SessionHandle::hangup_and_wait)
    /// when the caller needs to wait for the terminal API event.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let mut events = coordinator.events_for_session(&call_id).await?;
    /// coordinator.hangup(&call_id).await?;
    /// // Wait for Event::CallEnded / CallFailed / CallCancelled if needed.
    /// # let _ = events.next().await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn hangup(&self, session_id: &SessionId) -> Result<()> {
        // One exact SessionStateCell owns one retained hangup operation. The
        // public caller is only a waiter: dropping it cannot cancel dispatch,
        // wire confirmation, or finalization, and another caller joins the
        // same completion instead of crossing a shared generation fence.
        let hangup_control = self.helpers.state_machine.store.hangup_control(session_id);
        let Some((handle, control)) = hangup_control else {
            return Err(SessionError::SessionNotFound(format!(
                "Session {} has no current exact hangup lifetime",
                session_id.0
            )));
        };
        let coordinator = self
            .self_weak
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| {
                SessionError::InternalError("exact hangup supervisor is unavailable".to_string())
            })?;
        if !control.try_start() {
            drop(coordinator);
            return shared_hangup_completion_result(control.wait_for_completion().await);
        }

        let retained_control = Arc::clone(&control);
        let retained_session_id = session_id.clone();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let completion_guard = RetainedHangupTaskCompletion::new(retained_control);
        let hangup_scheduler = Arc::clone(&self.setup_teardown_scheduler);
        let spawned = hangup_scheduler.spawn_lifecycle_task(async move {
            let mut completion_guard = completion_guard;
            let result = if coordinator
                .helpers
                .state_machine
                .store
                .lifecycle_handle(&retained_session_id)
                .as_ref()
                != Some(&handle)
            {
                Err(SessionError::SessionNotFound(format!(
                    "Session {} exact lifetime is no longer current",
                    retained_session_id.0
                )))
            } else {
                match std::panic::AssertUnwindSafe(
                    coordinator.hangup_serialized(&retained_session_id),
                )
                .catch_unwind()
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(SessionError::InternalError(
                        "exact hangup operation stopped unexpectedly".to_string(),
                    )),
                }
            };
            completion_guard.finish(result.is_ok());
            let _ = result_tx.send(result);
        });
        if !spawned {
            return shared_hangup_completion_result(control.wait_for_completion().await);
        }

        match result_rx.await {
            Ok(result) => result,
            Err(_) => shared_hangup_completion_result(control.wait_for_completion().await),
        }
    }

    async fn hangup_serialized(&self, session_id: &SessionId) -> Result<()> {
        let initial_state = self
            .helpers
            .state_machine
            .store
            .with_session(session_id, |session| session.call_state.clone())
            .ok();
        // Fence the per-session retained transaction before dispatch. A BYE
        // send can complete and retain its exact transaction, then lose the
        // final state-store revision race to a synchronous peer response. In
        // that case dispatch reports a bookkeeping error even though the wire
        // operation must still be joined and reclaimed below.
        let bye_generation_before_dispatch = self
            .dialog_adapter
            .outgoing_bye_generation(session_id)
            .unwrap_or(0);
        let dispatch = self.helpers.hangup(session_id).await;
        let retained_new_bye = matches!(initial_state, Some(CallState::Active))
            && self
                .dialog_adapter
                .has_outgoing_bye_after(session_id, bye_generation_before_dispatch);

        if dispatch.is_ok() || retained_new_bye {
            // The coordinator/state machine is now the sole protocol teardown
            // authority for this exact call. It may send CANCEL immediately,
            // defer it until a provisional response makes it legal, or ACK+BYE
            // a racing 2xx. Managed initial-INVITE release must therefore
            // retire local ownership without starting a competing CANCEL/BYE
            // loop.
            self.dialog_adapter
                .mark_initial_invite_protocol_teardown(session_id);
        }
        // An established local hangup is not acknowledged merely because a
        // concurrent dialog event has already published terminal lifecycle
        // evidence. The peer still owes this exact BYE transaction a
        // successful final response (including during simultaneous-BYE
        // glare). Await that response before the generic observed-terminal
        // shortcut below can convert teardown into success. The confirmation
        // helper always performs exact local reclamation on timeout/non-2xx,
        // while preserving the wire failure for the caller.
        if matches!(initial_state, Some(CallState::Active)) {
            return complete_established_bye_dispatch(
                dispatch,
                retained_new_bye,
                self.finalize_confirmed_local_bye(session_id, "Local hangup"),
            )
            .await;
        }
        dispatch?;
        if self.lifecycle_snapshot(session_id).await.terminal.is_some() {
            self.release_after_observed_terminal(session_id).await;
            return Ok(());
        }
        self.schedule_setup_teardown_timeout_if_current(
            session_id,
            SetupTeardownWatchdogKind::Cancellation,
        )
        .await;
        Ok(())
    }

    async fn release_after_observed_terminal(&self, session_id: &SessionId) {
        let Some(handle) = self
            .helpers
            .state_machine
            .store
            .lifecycle_handle(session_id)
        else {
            return;
        };
        let release_guard = crate::cleanup_diag::stage_guard(
            crate::cleanup_diag::CleanupStage::TerminalRelease,
            &session_id.0,
        );
        match release_exact_local_resources(
            Arc::clone(&self.helpers.state_machine.store),
            Arc::clone(&self.helpers),
            Arc::clone(&self.dialog_adapter),
            Arc::clone(&self.media_adapter),
            handle,
        )
        .await
        {
            Ok(()) => release_guard.finish_success(),
            Err(error) => {
                tracing::debug!(%error, "exact release after observed terminal was incomplete");
                release_guard.finish_failure();
            }
        }
    }

    /// Compensate an outbound setup that failed after session allocation but
    /// before `OutboundCallBuilder::send` could return a usable call id.
    /// Cleanup is deliberately best-effort and idempotent; the original
    /// dispatch error remains the public result.
    pub(crate) async fn rollback_outbound_setup(&self, session_id: &SessionId) {
        tracing::debug!(
            session_id = %session_id,
            "rolling back partially allocated outbound setup"
        );
        self.release_after_observed_terminal(session_id).await;
    }

    pub(crate) async fn schedule_outbound_setup_timeout(&self, session_id: &SessionId) {
        self.schedule_setup_teardown_timeout_if_current(
            session_id,
            SetupTeardownWatchdogKind::OutboundSetup,
        )
        .await;
    }

    pub(crate) async fn schedule_inbound_setup_timeout(&self, session_id: &SessionId) {
        self.schedule_setup_teardown_timeout_if_current(
            session_id,
            SetupTeardownWatchdogKind::InboundSetup,
        )
        .await;
    }

    pub(crate) async fn schedule_active_call_media_timeout_if_current(
        &self,
        session_id: &SessionId,
    ) {
        let no_media_timeout = Duration::from_secs(self.config.active_call_no_media_timeout_secs);
        let media_idle_timeout =
            Duration::from_secs(self.config.active_call_media_idle_timeout_secs);
        if no_media_timeout.is_zero() && media_idle_timeout.is_zero() {
            return;
        }

        let Ok((role, call_state, handle, entered_state_at)) = self
            .helpers
            .state_machine
            .store
            .with_session(session_id, |session| {
                (
                    session.role,
                    session.call_state.clone(),
                    session.lifecycle_handle.clone(),
                    session.entered_state_at,
                )
            })
        else {
            return;
        };
        if role != Role::UAS || call_state != CallState::Active {
            return;
        }
        let Some(handle) = handle else {
            return;
        };

        let session_id = session_id.clone();
        let state_machine = Arc::clone(&self.helpers.state_machine);
        let helpers = Arc::clone(&self.helpers);
        let dialog_adapter = Arc::clone(&self.dialog_adapter);
        let media_adapter = Arc::clone(&self.media_adapter);
        let publisher = self.app_event_publisher.clone();
        let watchdog_scheduler = Arc::clone(&self.setup_teardown_scheduler);
        let task_scheduler = Arc::clone(&watchdog_scheduler);
        let _ = watchdog_scheduler.spawn_lifecycle_task(async move {
            let Some(initial_packets_received) =
                media_adapter.rtp_packets_received(&session_id).await
            else {
                return;
            };
            let mut last_packets_received = initial_packets_received;
            let mut saw_media = initial_packets_received > 0;
            loop {
                let timeout = if saw_media {
                    media_idle_timeout
                } else if !no_media_timeout.is_zero() {
                    no_media_timeout
                } else {
                    media_idle_timeout
                };
                if timeout.is_zero() {
                    return;
                }

                if !task_scheduler.sleep_or_closed(timeout).await
                    || !task_scheduler.is_accepting()
                {
                    return;
                }

                let Ok(current) = state_machine.store.get_session_snapshot_exact(&handle) else {
                    return;
                };
                let current = current.state();
                if current.role != Role::UAS
                    || current.call_state != CallState::Active
                    || current.entered_state_at != entered_state_at
                {
                    return;
                }

                let Some(packets_received) = media_adapter.rtp_packets_received(&session_id).await
                else {
                    return;
                };

                if packets_received > last_packets_received {
                    saw_media = true;
                    last_packets_received = packets_received;
                    continue;
                }

                let reason = if saw_media {
                    format!(
                        "Active call released after RTP was idle for {}s",
                        timeout.as_secs()
                    )
                } else {
                    format!(
                        "Active call released after no RTP was received within {}s after answer",
                        timeout.as_secs()
                    )
                };

                tracing::warn!(
                    "active call media watchdog firing for session {} after {:?}: last_packets_received={}, saw_media={}",
                    session_id,
                    timeout,
                    last_packets_received,
                    saw_media
                );

                if let Err(err) = state_machine
                    .process_event(&session_id, EventType::HangupCall)
                    .await
                {
                    tracing::warn!(
                        "active call media watchdog failed to process HangupCall for {} in state {:?}: {}",
                        session_id,
                        current.call_state,
                        err
                    );
                }

                let release_guard = crate::cleanup_diag::stage_guard(
                    crate::cleanup_diag::CleanupStage::TerminalRelease,
                    &session_id.0,
                );
                let claim_owner = match publisher.claim_exact_terminal(&handle) {
                    ExactTerminalClaim::Owner(owner) => owner,
                    ExactTerminalClaim::Observer(_) => {
                        release_guard.finish_success();
                        return;
                    }
                };
                let api_event = crate::api::events::Event::CallEnded {
                    call_id: session_id.clone(),
                    reason,
                };
                let outcome = publisher
                    .publish_terminal_then_release(
                        api_event,
                        release_exact_local_resources(
                            Arc::clone(&state_machine.store),
                            helpers,
                            dialog_adapter,
                            media_adapter,
                            handle,
                        ),
                    )
                    .await;
                let publication_succeeded = match outcome.publication {
                    Ok(()) => true,
                    Err(err) => {
                        tracing::warn!(
                            "active call media watchdog failed to publish terminal event for {}: {}",
                            session_id,
                            err
                        );
                        false
                    }
                };
                if let Err(error) = outcome.release {
                    tracing::debug!(%error, "active media watchdog exact release was incomplete");
                    release_guard.finish_failure();
                    claim_owner.finish(if publication_succeeded {
                        ExactTerminalCompletion::ReleaseFailed
                    } else {
                        ExactTerminalCompletion::PublicationAndReleaseFailed
                    });
                    return;
                }
                release_guard.finish_success();
                claim_owner.finish(if publication_succeeded {
                    ExactTerminalCompletion::PublishedAndReleased
                } else {
                    ExactTerminalCompletion::PublicationFailed
                });
                return;
            }
        });
    }

    pub(crate) fn setup_teardown_timeout_duration(&self) -> Duration {
        Duration::from_secs(self.config.setup_teardown_timeout_secs)
    }

    async fn schedule_setup_teardown_timeout_if_current(
        &self,
        session_id: &SessionId,
        kind: SetupTeardownWatchdogKind,
    ) {
        let timeout = Duration::from_secs(self.config.setup_teardown_timeout_secs);
        if timeout.is_zero() {
            return;
        }

        let Ok((call_state, handle, entered_state_at)) = self
            .helpers
            .state_machine
            .store
            .with_session(session_id, |session| {
                (
                    session.call_state.clone(),
                    session.lifecycle_handle.clone(),
                    session.entered_state_at,
                )
            })
        else {
            return;
        };
        if !kind.watched_states().contains(&call_state) {
            return;
        }
        let Some(handle) = handle else {
            return;
        };
        let Some(deadline) = Instant::now().checked_add(timeout) else {
            tracing::error!(
                timeout_secs = self.config.setup_teardown_timeout_secs,
                "setup teardown watchdog deadline overflow"
            );
            return;
        };

        let _ = self
            .setup_teardown_scheduler
            .schedule(deadline, handle, entered_state_at, kind);
    }

    fn setup_teardown_deadline_is_current(&self, deadline: &SetupTeardownDeadline) -> bool {
        self.helpers
            .state_machine
            .store
            .get_session_snapshot_exact(&deadline.handle)
            .is_ok_and(|current| {
                deadline
                    .kind
                    .watched_states()
                    .contains(&current.state().call_state)
                    && current.state().entered_state_at == deadline.entered_state_at
            })
    }

    fn dispatch_setup_teardown_deadline(
        self: &Arc<Self>,
        deadline: SetupTeardownDeadline,
        fire_permit: tokio::sync::OwnedSemaphorePermit,
    ) -> bool {
        let coordinator = Arc::clone(self);
        self.setup_teardown_scheduler
            .spawn_deadline_fire(async move {
                let _fire_permit = fire_permit;
                coordinator.fire_setup_teardown_deadline(deadline).await;
            })
    }

    async fn fire_setup_teardown_deadline(&self, deadline: SetupTeardownDeadline) {
        // Revalidate after task admission. The session can complete between
        // the scheduler's cheap preflight and this cold handler starting.
        let Ok(current) = self
            .helpers
            .state_machine
            .store
            .get_session_snapshot_exact(&deadline.handle)
        else {
            crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
            return;
        };
        let current_state = current.state().call_state;
        if !deadline.kind.watched_states().contains(&current_state)
            || current.state().entered_state_at != deadline.entered_state_at
        {
            crate::cleanup_diag::record_setup_teardown_watchdog_disarmed();
            return;
        }

        #[cfg(test)]
        self.setup_teardown_scheduler
            .pause_if_requested_for_test()
            .await;

        crate::cleanup_diag::record_setup_teardown_watchdog_fired();
        let session_id = deadline.handle.session_id().clone();
        let reason = deadline.kind.reason();
        tracing::warn!(
            "setup teardown watchdog firing for session {} in state {:?}: {}",
            session_id,
            current_state,
            reason
        );

        let state_machine = Arc::clone(&self.helpers.state_machine);
        let helpers = Arc::clone(&self.helpers);
        let dialog_adapter = Arc::clone(&self.dialog_adapter);
        let media_adapter = Arc::clone(&self.media_adapter);
        let publisher = self.app_event_publisher.clone();
        if let Err(err) = state_machine
            .process_event(&session_id, EventType::DialogTimeout)
            .await
        {
            tracing::warn!(
                "setup teardown watchdog failed for session {} in state {:?}: {}",
                session_id,
                current_state,
                err
            );
            crate::cleanup_diag::record_setup_teardown_watchdog_transition_failed();
            return;
        }

        let Ok(after) = state_machine
            .store
            .get_session_snapshot_exact(&deadline.handle)
        else {
            crate::cleanup_diag::record_setup_teardown_watchdog_transition_failed();
            return;
        };
        if !after.state().call_state.is_final() {
            crate::cleanup_diag::record_setup_teardown_watchdog_transition_failed();
            return;
        }

        let release_guard = crate::cleanup_diag::stage_guard(
            crate::cleanup_diag::CleanupStage::TerminalRelease,
            &session_id.0,
        );
        let claim_owner = match publisher.claim_exact_terminal(&deadline.handle) {
            ExactTerminalClaim::Owner(owner) => owner,
            ExactTerminalClaim::Observer(_) => {
                release_guard.finish_success();
                crate::cleanup_diag::record_setup_teardown_watchdog_release_completed();
                return;
            }
        };
        let api_event = match deadline.kind.terminal() {
            SetupTeardownTimeoutTerminal::Cancelled => crate::api::events::Event::CallCancelled {
                call_id: session_id.clone(),
            },
            SetupTeardownTimeoutTerminal::Failed => crate::api::events::Event::CallFailed {
                call_id: session_id.clone(),
                status_code: 408,
                reason: reason.to_string(),
            },
        };
        let outcome = publisher
            .publish_terminal_then_release(
                api_event,
                release_exact_local_resources(
                    Arc::clone(&state_machine.store),
                    helpers,
                    dialog_adapter,
                    media_adapter,
                    deadline.handle,
                ),
            )
            .await;
        let publication_succeeded = match outcome.publication {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!(
                    "setup teardown watchdog failed to publish terminal event for {}: {}",
                    session_id,
                    err
                );
                false
            }
        };
        if let Err(error) = outcome.release {
            tracing::debug!(%error, "setup watchdog exact release was incomplete");
            release_guard.finish_failure();
            crate::cleanup_diag::record_setup_teardown_watchdog_release_failed();
            claim_owner.finish(if publication_succeeded {
                ExactTerminalCompletion::ReleaseFailed
            } else {
                ExactTerminalCompletion::PublicationAndReleaseFailed
            });
            return;
        }
        release_guard.finish_success();
        crate::cleanup_diag::record_setup_teardown_watchdog_release_completed();
        claim_owner.finish(if publication_succeeded {
            ExactTerminalCompletion::PublishedAndReleased
        } else {
            ExactTerminalCompletion::PublicationFailed
        });
    }

    /// Complete local BYE teardown when rvoip-sip-dialog accepts the outbound BYE.
    ///
    /// Dialog-core does not currently emit a per-session `DialogTerminated`
    /// event for every successful outbound BYE transaction, so the public BYE
    /// paths release rvoip-sip state explicitly after the BYE has been
    /// dispatched. Inbound BYE/CANCEL/failure paths still release through the
    /// dialog event handler.
    pub(crate) async fn finalize_local_bye(
        &self,
        session_id: &SessionId,
        reason: impl Into<String>,
    ) -> Result<()> {
        let Some(handle) = self
            .helpers
            .state_machine
            .store
            .lifecycle_handle(session_id)
        else {
            return Ok(());
        };
        let publication_timeout = self.dialog_adapter.non_invite_transaction_timeout();

        let claim_owner = match self.app_event_publisher.claim_exact_terminal(&handle) {
            ExactTerminalClaim::Owner(owner) => {
                tracing::debug!(session = %session_id, "local BYE finalizer owns exact terminal publication");
                owner
            }
            ExactTerminalClaim::Observer(observer) => {
                tracing::debug!(session = %session_id, "local BYE finalizer is joining exact terminal publication");
                match tokio::time::timeout(publication_timeout, observer.wait()).await {
                    Ok(
                        ExactTerminalCompletion::PublishedAndReleased
                        | ExactTerminalCompletion::PublicationFailed,
                    ) => return Ok(()),
                    Ok(completion) => {
                        tracing::warn!(
                            session = %session_id,
                            ?completion,
                            "local BYE terminal owner did not release exact resources; taking over cleanup"
                        );
                    }
                    Err(_) => {
                        tracing::warn!(
                            session = %session_id,
                            timeout_ms = publication_timeout.as_millis(),
                            "local BYE terminal owner exceeded its deadline; taking over exact cleanup"
                        );
                    }
                }
                let release_guard = crate::cleanup_diag::stage_guard(
                    crate::cleanup_diag::CleanupStage::TerminalRelease,
                    &session_id.0,
                );
                let release = release_exact_local_resources_with_retry(
                    Arc::clone(&self.helpers.state_machine.store),
                    Arc::clone(&self.helpers),
                    Arc::clone(&self.dialog_adapter),
                    Arc::clone(&self.media_adapter),
                    handle,
                )
                .await;
                match release {
                    Ok(()) => release_guard.finish_success(),
                    Err(_) => release_guard.finish_failure(),
                }
                return release;
            }
        };

        let release_guard = crate::cleanup_diag::stage_guard(
            crate::cleanup_diag::CleanupStage::TerminalRelease,
            &session_id.0,
        );

        let api_event = crate::api::events::Event::CallEnded {
            call_id: session_id.clone(),
            reason: reason.into(),
        };
        let release_handle = handle.clone();
        let outcome = self
            .app_event_publisher
            .publish_terminal_then_release_bounded(
                api_event,
                release_exact_local_resources_with_retry(
                    Arc::clone(&self.helpers.state_machine.store),
                    Arc::clone(&self.helpers),
                    Arc::clone(&self.dialog_adapter),
                    Arc::clone(&self.media_adapter),
                    release_handle,
                ),
                publication_timeout,
            )
            .await;
        let publication_succeeded = match outcome.publication {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!(
                    error_class = "app-event-publication",
                    "Local BYE terminal event publication failed after lifecycle admission: {}",
                    err
                );
                false
            }
        };
        if let Err(error) = outcome.release {
            release_guard.finish_failure();
            claim_owner.finish(if publication_succeeded {
                ExactTerminalCompletion::ReleaseFailed
            } else {
                ExactTerminalCompletion::PublicationAndReleaseFailed
            });
            return Err(error);
        }
        release_guard.finish_success();
        claim_owner.finish(if publication_succeeded {
            ExactTerminalCompletion::PublishedAndReleased
        } else {
            ExactTerminalCompletion::PublicationFailed
        });
        Ok(())
    }

    /// Await the peer's successful final response before acknowledging local
    /// BYE teardown. Local resources are reclaimed on every outcome, but a
    /// timeout or non-2xx remains visible to the caller as wire failure.
    pub(crate) async fn finalize_confirmed_local_bye(
        &self,
        session_id: &SessionId,
        reason: impl Into<String>,
    ) -> Result<()> {
        let confirmation = self
            .dialog_adapter
            .wait_for_outgoing_bye_final_response(session_id)
            .await;
        let finalization = self.finalize_local_bye(session_id, reason).await;
        match confirmation {
            Ok(()) => finalization,
            Err(error) => {
                if let Err(cleanup_error) = finalization {
                    tracing::debug!(
                        %cleanup_error,
                        "local SIP cleanup was incomplete after BYE confirmation failure"
                    );
                }
                Err(error)
            }
        }
    }

    /// Begin authoritative local reclamation without publishing an app event.
    ///
    /// Normal teardown uses [`Self::finalize_local_bye`] so terminal delivery
    /// linearizes before the session disappears. This fallback exists only
    /// for a blocked or failed terminal publisher. It captures the exact
    /// authority/registry owner synchronously; the returned cleanup is then
    /// run as retained work and performs quiesce, lower release, and exact
    /// conditional removal as one ordered continuation.
    pub(crate) async fn begin_force_reclaim_local_session(
        &self,
        session_id: &SessionId,
    ) -> ForcedLocalSessionCleanup {
        let release_guard = crate::cleanup_diag::stage_guard(
            crate::cleanup_diag::CleanupStage::TerminalRelease,
            &session_id.0,
        );

        let session_store = Arc::clone(&self.helpers.state_machine.store);
        let handle = session_store.lifecycle_handle(session_id);
        release_guard.finish_success();
        ForcedLocalSessionCleanup {
            handle,
            helpers: Arc::clone(&self.helpers),
            dialog_adapter: Arc::clone(&self.dialog_adapter),
            media_adapter: Arc::clone(&self.media_adapter),
            session_store,
        }
    }

    /// Bridge the RTP streams of two active sessions at the media layer.
    ///
    /// Transparent packet-level relay: inbound RTP from session A is
    /// forwarded as outbound RTP on session B and vice versa, without
    /// transcoding. Intended for b2bua-style consumers that need to connect
    /// two SIP legs without shuffling AudioFrames through app code.
    ///
    /// # Preconditions
    ///
    /// - Both sessions must exist and be in `CallState::Active` (i.e. have
    ///   a negotiated remote RTP address).
    /// - Both sessions must have negotiated the same codec payload type.
    ///   Codec mismatch returns [`BridgeError::CodecMismatch`].
    /// - Neither session may already be bridged.
    ///
    /// Dropping the returned [`BridgeHandle`] tears the bridge down. DTMF
    /// (RFC 2833) rides the RTP stream and is forwarded transparently;
    /// RTCP is not bridged — each leg keeps generating its own reports.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, a: rvoip_sip::SessionId, b: rvoip_sip::SessionId) -> Result<(), rvoip_sip::BridgeError> {
    /// let bridge = coordinator.bridge(&a, &b).await?;
    /// // Keep `bridge` alive for as long as the RTP relay should run.
    /// drop(bridge);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bridge(
        &self,
        session_a: &SessionId,
        session_b: &SessionId,
    ) -> std::result::Result<BridgeHandle, BridgeError> {
        self.media_adapter
            .bridge_rtp_sessions(session_a, session_b)
            .await
    }

    /// Send a reliable 183 Session Progress with early-media SDP (RFC 3262).
    ///
    /// - `sdp: Some(body)` sends the supplied SDP verbatim.
    /// - `sdp: None` generates an answer from the stored remote offer via
    ///   `MediaAdapter::negotiate_sdp_as_uas` (same path as `accept_call`).
    ///
    /// Fails fast with `UnreliableProvisionalsNotSupported` when the peer
    /// did not advertise `Supported: 100rel` on the INVITE. Transitions the
    /// session to `CallState::EarlyMedia`. Valid from `Ringing` and
    /// `EarlyMedia` (re-emission updates the SDP and bumps `RSeq`).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, incoming: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.send_early_media(&incoming, None).await?;
    /// coordinator.accept_call(&incoming).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_early_media(
        &self,
        session_id: &SessionId,
        sdp: Option<String>,
    ) -> Result<()> {
        if !self.dialog_adapter.peer_supports_100rel(session_id).await? {
            return Err(SessionError::UnreliableProvisionalsNotSupported);
        }
        self.helpers.send_early_media(session_id, sdp).await
    }

    /// Swap the audio source on the running transmitter for a session.
    ///
    /// Typical use: after [`send_early_media`][Self::send_early_media] has
    /// put the session into `EarlyMedia` (which starts a pass-through
    /// transmitter by default), call this to replace silence with a
    /// ringback tone, a "please hold" WAV, or any other
    /// [`AudioSource`] variant.
    ///
    /// On transition to `Active` (after `accept_call`), the state machine
    /// automatically swaps the transmitter back to `AudioSource::PassThrough`
    /// so bidirectional audio flows without further action from the app.
    /// Apps that want a *different* source after answer (e.g., continued
    /// announcement playback over an active call) should call this method
    /// again *after* the `CallEstablished` event fires.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// use rvoip_sip::AudioSource;
    ///
    /// coordinator.set_audio_source(
    ///     &call_id,
    ///     AudioSource::Tone { frequency: 440.0, amplitude: 0.4 },
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_audio_source(
        &self,
        session_id: &SessionId,
        source: AudioSource,
    ) -> Result<()> {
        self.media_adapter
            .set_audio_source(session_id, source)
            .await
    }

    /// Put a call on hold.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.hold(&call_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn hold(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::HoldCall)
            .await?;
        Ok(())
    }

    /// Resume a call from hold.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.resume(&call_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::ResumeCall)
            .await?;
        Ok(())
    }

    // ===== Conference Operations =====

    /// Create a conference from an active call.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, host: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.create_conference(&host, "support-bridge").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_conference(&self, session_id: &SessionId, name: &str) -> Result<()> {
        self.helpers.create_conference(session_id, name).await
    }

    /// Add a participant to a conference hosted by another active session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, host: rvoip_sip::SessionId, participant: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.add_to_conference(&host, &participant).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn add_to_conference(
        &self,
        host_session_id: &SessionId,
        participant_session_id: &SessionId,
    ) -> Result<()> {
        self.helpers
            .add_to_conference(host_session_id, participant_session_id)
            .await
    }

    /// Join an existing conference by conference id.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.join_conference(&call_id, "support-bridge").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn join_conference(&self, session_id: &SessionId, conference_id: &str) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(
                session_id,
                EventType::JoinConference {
                    conference_id: conference_id.to_string(),
                },
            )
            .await?;
        Ok(())
    }

    // ===== Event System Integration =====
    // Callback registry removed - using event-driven approach via SimplePeer

    /// Materialize a [`SessionHandle`](crate::api::handle::SessionHandle)
    /// for an existing call_id.
    ///
    /// Returns a handle for invoking control APIs (hangup, hold, resume,
    /// DTMF, …) on a session created via the canonical builder chain
    /// (`coord.invite(...).send()` returns a [`CallId`](crate::api::handle::CallId);
    /// pair it with this helper to get the rich `SessionHandle` for
    /// in-call control).
    pub fn session(
        self: &Arc<Self>,
        call_id: &crate::api::handle::CallId,
    ) -> crate::api::handle::SessionHandle {
        crate::api::handle::SessionHandle::new(call_id.clone(), self.clone())
    }

    /// Terminate the current session tracked by the session store.
    ///
    /// This is an advanced compatibility helper for single-session flows. New
    /// code should usually hold the specific [`SessionId`] and call
    /// [`hangup`](Self::hangup).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// coordinator.terminate_current_session().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn terminate_current_session(&self) -> Result<()> {
        // Get the current session ID
        if let Some(session_id) = self
            .helpers
            .state_machine
            .store
            .get_current_session_id()
            .await
        {
            self.hangup(&session_id).await
        } else {
            Ok(()) // No session to terminate
        }
    }

    /// Accept a pending inbound REFER request and send RFC 3515 acceptance
    /// responses/NOTIFYs through the state machine.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// // Call this after receiving Event::ReferReceived for `call_id`.
    /// coordinator.accept_refer(&call_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept_refer(&self, session_id: &SessionId) -> Result<()> {
        let (refer_to, transaction_id) =
            self.helpers
                .state_machine
                .store
                .with_session(session_id, |session| {
                    let refer_to = session.transfer_target.clone().ok_or_else(|| {
                        SessionError::Other(format!(
                            "No pending REFER target for session {}",
                            session_id
                        ))
                    })?;
                    let transaction_id = session.refer_transaction_id.clone().ok_or_else(|| {
                        SessionError::Other(format!(
                            "No pending REFER transaction for session {}",
                            session_id
                        ))
                    })?;
                    Ok::<_, SessionError>((refer_to, transaction_id))
                })??;

        self.helpers
            .state_machine
            .process_event(
                session_id,
                EventType::TransferRequested {
                    refer_to,
                    transfer_type: "blind".to_string(),
                    transaction_id: transaction_id.clone(),
                },
            )
            .await?;

        if self
            .helpers
            .state_machine
            .store
            .with_session(session_id, |_| ())
            .is_ok()
        {
            self.helpers
                .state_machine
                .store
                .update_session_with(session_id, |session| {
                    if session.refer_transaction_id.as_deref() == Some(transaction_id.as_str()) {
                        session.refer_transaction_id = None;
                    }
                })
                .await?;
        }

        Ok(())
    }

    /// Reject a pending inbound REFER request with a final response.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.reject_refer(&call_id, 603, "Decline").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn reject_refer(
        &self,
        session_id: &SessionId,
        status_code: u16,
        reason: &str,
    ) -> Result<()> {
        let transaction_id =
            self.helpers
                .state_machine
                .store
                .with_session(session_id, |session| {
                    session.refer_transaction_id.clone().ok_or_else(|| {
                        SessionError::Other(format!(
                            "No pending REFER transaction for session {}",
                            session_id
                        ))
                    })
                })??;

        let event = rvoip_infra_common::events::cross_crate::RvoipCrossCrateEvent::SessionToDialog(
            rvoip_infra_common::events::cross_crate::SessionToDialogEvent::ReferResponse {
                transaction_id: transaction_id.clone(),
                accept: false,
                status_code,
                reason: reason.to_string(),
            },
        );

        self.global_coordinator
            .publish(Arc::new(event))
            .await
            .map_err(|e| {
                SessionError::Other(format!("Failed to publish REFER rejection: {}", e))
            })?;

        if self
            .helpers
            .state_machine
            .store
            .with_session(session_id, |_| ())
            .is_ok()
        {
            self.helpers
                .state_machine
                .store
                .update_session_with(session_id, |session| {
                    if session.refer_transaction_id.as_deref() == Some(transaction_id.as_str()) {
                        session.refer_transaction_id = None;
                        session.transfer_target = None;
                        session.transfer_state = crate::session_store::state::TransferState::None;
                    }
                })
                .await?;
        }

        Ok(())
    }

    /// Send a REFER progress NOTIFY with a SIP status code and reason.
    ///
    /// This is the low-level helper for custom REFER orchestration. Transfer
    /// legs created with [`make_transfer_leg`](Self::make_transfer_leg)
    /// emit ordinary REFER progress automatically.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.send_refer_notify(&call_id, 180, "Ringing").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_refer_notify(
        &self,
        session_id: &SessionId,
        status_code: u16,
        reason: &str,
    ) -> Result<()> {
        self.dialog_adapter
            .send_refer_notify(session_id, status_code, reason)
            .await
    }

    /// Fetch the SIP-level identity (`Call-ID`, local/remote tags) of a
    /// session's dialog. Returns `None` if the dialog isn't established
    /// yet or has already been cleaned up.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// if let Some(identity) = coordinator.dialog_identity(&call_id).await? {
    ///     if let Some(replaces) = identity.to_replaces_value() {
    ///         println!("Replaces value: {replaces}");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn dialog_identity(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<crate::api::types::DialogIdentity>> {
        self.dialog_adapter.dialog_identity(session_id).await
    }

    // ===== DTMF Operations =====

    /// Send a single RFC 4733 DTMF digit over the active media session
    /// at a 100 ms default duration (suitable for interactive softphone
    /// use).
    ///
    /// Goes directly through [`MediaAdapter::send_dtmf_rfc4733`] rather
    /// than the state machine: DTMF is an in-call side-effect, not a
    /// state transition, and the state table does not (intentionally)
    /// enumerate a SendDTMF transition. The media adapter resolves
    /// `session_id → dialog_id`, encodes the RFC 4733 telephone-event
    /// payload, and transmits with PT 101 over the existing RTP
    /// session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.send_dtmf(&call_id, '5').await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_dtmf(&self, session_id: &SessionId, digit: char) -> Result<()> {
        self.media_adapter
            .send_dtmf_rfc4733(session_id, digit, 100)
            .await
    }

    /// Send a bounded RFC 4733 sequence with caller-selected tone duration
    /// and quiet time between digits.
    ///
    /// The complete sequence is validated before any RTP is emitted. This is
    /// the transport-specific primitive used by the generic connection
    /// adapter; single-digit callers may continue using [`Self::send_dtmf`].
    pub async fn send_dtmf_sequence(
        &self,
        session_id: &SessionId,
        digits: &str,
        duration_ms: u32,
        inter_digit_ms: u32,
    ) -> Result<()> {
        self.media_adapter
            .send_dtmf_sequence_rfc4733(session_id, digits, duration_ms, inter_digit_ms)
            .await
    }

    // ===== Recording Operations =====

    /// Start recording a call.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.start_recording(&call_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_recording(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::StartRecording)
            .await?;
        Ok(())
    }

    /// Stop recording a call.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.stop_recording(&call_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::StopRecording)
            .await?;
        Ok(())
    }

    // ===== Query Operations =====

    /// Get detailed session information.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let info = coordinator.get_session_info(&call_id).await?;
    /// println!("session state: {:?}", info.state);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_session_info(&self, session_id: &SessionId) -> Result<SessionInfo> {
        self.helpers.get_session_info(session_id).await
    }

    /// List all active sessions.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) {
    /// let sessions = coordinator.list_sessions().await;
    /// println!("active sessions: {}", sessions.len());
    /// # }
    /// ```
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        self.helpers.list_sessions().await
    }

    /// Get the current state of a session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let state = coordinator.get_state(&call_id).await?;
    /// println!("call state: {state:?}");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_state(&self, session_id: &SessionId) -> Result<CallState> {
        self.helpers.get_state(session_id).await
    }

    /// Check whether a session is in a conference.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// if coordinator.is_in_conference(&call_id).await? {
    ///     println!("call is in a conference");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_in_conference(&self, session_id: &SessionId) -> Result<bool> {
        self.helpers.is_in_conference(session_id).await
    }

    // ===== Audio Operations =====

    /// Subscribe to decoded audio frames for a session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let mut audio = coordinator.subscribe_to_audio(&call_id).await?;
    /// tokio::spawn(async move {
    ///     while let Some(frame) = audio.receiver.recv().await {
    ///         println!("received {} samples", frame.samples.len());
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_to_audio(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::types::AudioFrameSubscriber> {
        self.media_adapter
            .subscribe_to_audio_frames(session_id)
            .await
    }

    /// Send an encoded/decoded audio frame to a session's media path.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let frame = rvoip_media_core::types::AudioFrame::new(vec![0i16; 160], 8000, 1, 0);
    /// coordinator.send_audio(&call_id, frame).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_audio(&self, session_id: &SessionId, frame: AudioFrame) -> Result<()> {
        self.media_adapter.send_audio_frame(session_id, frame).await
    }

    /// Resolve the exact SDP-negotiated media format for a live session.
    ///
    /// This is crate-private because transport-neutral callers consume the
    /// resulting [`crate::media_stream::SipMediaStream`] codec descriptor.
    pub(crate) async fn negotiated_media_config(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<crate::session_store::state::NegotiatedConfig>> {
        self.helpers.negotiated_media_config(session_id).await
    }

    // ===== Event Subscriptions =====

    /// Subscribe a callback to low-level state-machine session events.
    ///
    /// This is an advanced compatibility hook. New application code should
    /// prefer [`events`](Self::events) or [`events_for_session`](Self::events_for_session).
    ///
    /// Renamed from `subscribe(...)` per SIP_API_DESIGN_2.md Phase 12: the
    /// bare `subscribe` entry now names the SUBSCRIBE-method verb-builder.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) {
    /// coordinator.on_session_events(call_id, |_event| {
    ///     // Observe low-level state-machine events.
    /// }).await;
    /// # }
    /// ```
    pub async fn on_session_events<F>(&self, session_id: SessionId, callback: F)
    where
        F: Fn(crate::state_machine::helpers::SessionEvent) + Send + Sync + 'static,
    {
        self.helpers.subscribe(session_id, callback).await
    }

    /// Unsubscribe from low-level state-machine events for a session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) {
    /// coordinator.unsubscribe(&call_id).await;
    /// # }
    /// ```
    pub async fn unsubscribe(&self, session_id: &SessionId) {
        self.helpers.unsubscribe(session_id).await
    }

    // ===== Incoming Call Handling =====

    /// Get the next low-level incoming call notification.
    ///
    /// This is the coordinator primitive underneath
    /// [`StreamPeer::wait_for_incoming`](crate::api::stream_peer::StreamPeer::wait_for_incoming).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) {
    /// if let Some(incoming) = coordinator.get_incoming_call().await {
    ///     println!("incoming call from {}", incoming.from);
    /// }
    /// # }
    /// ```
    pub async fn get_incoming_call(&self) -> Option<IncomingCallInfo> {
        self.incoming_rx.write().await.recv().await
    }

    /// Wait for the next incoming INVITE and return the controllable
    /// [`IncomingCall`](crate::IncomingCall) wrapper.
    ///
    /// Open this wait before the caller sends the INVITE. Like
    /// [`events`](Self::events), incoming-call notifications are broadcast
    /// events and are not replayed to late subscribers.
    ///
    /// This is the UnifiedCoordinator equivalent of
    /// [`StreamPeer::wait_for_incoming`](crate::StreamPeer::wait_for_incoming)
    /// for applications that want direct coordinator control plus inbound auth
    /// helpers such as
    /// [`IncomingCall::authenticate_with`](crate::IncomingCall::authenticate_with).
    pub async fn wait_for_incoming_call(
        self: &Arc<Self>,
    ) -> Result<crate::api::incoming::IncomingCall> {
        let mut events = self.events().await?;
        self.next_incoming_call(&mut events)
            .await?
            .ok_or_else(|| SessionError::Other("Event channel closed".to_string()))
    }

    /// Return the next incoming INVITE from an existing event receiver.
    ///
    /// Use this when a server may challenge a request and then receive an
    /// immediate retry. Keeping one receiver open avoids missing the retry
    /// event between separate subscriptions.
    pub async fn next_incoming_call(
        self: &Arc<Self>,
        events: &mut crate::api::stream_peer::EventReceiver,
    ) -> Result<Option<crate::api::incoming::IncomingCall>> {
        let Some((call_id, from, to, sdp)) = events.next_incoming().await else {
            return Ok(None);
        };

        let pending = self.pending_incoming_bundle_exact(&call_id);
        let parsed = pending.as_ref().and_then(|bundle| bundle.request.clone());
        let transport = pending.and_then(|bundle| bundle.transport);
        let incoming = match parsed {
            Some(req) => crate::api::incoming::IncomingCall::with_request(
                call_id,
                from,
                to,
                sdp,
                self.clone(),
                req,
            ),
            None => crate::api::incoming::IncomingCall::new(call_id, from, to, sdp, self.clone()),
        }
        .with_transport_context(
            transport
                .as_deref()
                .cloned()
                .unwrap_or_else(crate::auth::SipTransportSecurityContext::unknown),
        );
        Ok(Some(incoming))
    }

    /// Resolve pending inbound material through the exact session generation
    /// and registry slot associated with this call. Ambiguous or quiescing raw
    /// identifiers fail closed.
    pub(crate) fn pending_incoming_bundle_exact(
        &self,
        session_id: &SessionId,
    ) -> Option<PendingInboundBundle> {
        let handle = self
            .helpers
            .state_machine
            .store
            .lifecycle_handle(session_id)?;
        self.session_registry
            .pending_bundle_exact(handle.key(), handle.slot_revision())
            .ok()
    }

    // ===== Auto-Transfer Handling =====

    /// Enable automatic blind transfer handling - DISABLED
    /// Auto-transfer now handled in SessionEventHandler to avoid event stealing
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) {
    /// coordinator.enable_auto_transfer();
    /// # }
    /// ```
    pub fn enable_auto_transfer(self: &Arc<Self>) {
        tracing::info!("🔄 Auto-transfer: handled by SessionEventHandler");
    }

    // extract_field method removed - no longer needed without transfer coordinator

    // ===== Server-Side Registration =====

    /// Start server-side registration handling
    ///
    /// This creates and starts a RegistrationAdapter that handles incoming REGISTER
    /// requests via the global event bus. The registrar service authenticates users
    /// and manages registrations.
    ///
    /// # Arguments
    /// * `realm` - The SIP realm for digest authentication (e.g., "example.com")
    /// * `users` - Map of username -> password for authentication
    ///
    /// # Returns
    ///
    /// `Arc<RegistrarService>` for inspecting and managing registrations.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// let users = std::collections::HashMap::from([
    ///     ("alice".to_string(), "secret".to_string()),
    /// ]);
    /// let registrar = coordinator.start_registration_server("example.com", users).await?;
    /// # let _ = registrar;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_registration_server(
        &self,
        realm: &str,
        users: std::collections::HashMap<String, String>,
    ) -> Result<Arc<rvoip_sip_registrar::RegistrarService>> {
        use crate::adapters::RegistrationAdapter;
        use rvoip_sip_registrar::{api::ServiceMode, types::RegistrarConfig, RegistrarService};

        tracing::info!(
            "🔐 Starting server-side registration handler with realm: {}",
            realm
        );

        // Create registrar service with authentication
        let registrar =
            RegistrarService::with_auth(ServiceMode::B2BUA, RegistrarConfig::default(), realm)
                .await
                .map_err(|e| {
                    SessionError::InternalError(format!("Failed to create registrar: {}", e))
                })?;

        // Add users to the registrar
        if let Some(user_store) = registrar.user_store() {
            for (username, password) in users {
                user_store.add_user(&username, &password).map_err(|e| {
                    SessionError::InternalError(format!("Failed to add user: {}", e))
                })?;
                tracing::debug!("Added user: {}", username);
            }
        }

        let registrar = Arc::new(registrar);

        // Subscribe the registration adapter to THIS coordinator's event bus — the
        // same instance the dialog/transaction layer publishes inbound REGISTER to.
        // Using the process-wide singleton here meant the adapter never saw REGISTER,
        // so the registrar never answered and clients timed out.
        let global_coordinator = self.global_coordinator.clone();

        // Create and start the registration adapter
        let adapter = Arc::new(RegistrationAdapter::new(
            registrar.clone(),
            global_coordinator,
        ));

        adapter.start().await.map_err(|e| {
            SessionError::InternalError(format!("Failed to start registration adapter: {}", e))
        })?;

        tracing::info!("✅ Server-side registration handler started");

        Ok(registrar)
    }

    // ===== Internal Helpers =====

    async fn create_dialog_api(
        config: &Config,
        global_coordinator: Arc<GlobalEventCoordinator>,
        sip_trace_owner_id: Option<String>,
        listener_auth_policy: crate::auth::SipListenerAuthPolicy,
    ) -> Result<Arc<rvoip_sip_dialog::api::unified::UnifiedDialogApi>> {
        use rvoip_sip_dialog::api::unified::UnifiedDialogApi;
        use rvoip_sip_dialog::config::DialogManagerConfig;
        use rvoip_sip_dialog::transaction::{
            transport::{TransportManager, TransportManagerConfig},
            TransactionManager,
        };

        // Create transport manager first (rvoip-sip-dialog's own transport manager).
        //
        // TCP is enabled by default — the URI-aware
        // `MultiplexedTransport` (`crates/sip/rvoip-sip-dialog/src/transaction/transport/multiplexed.rs`)
        // routes outbound INVITEs to the right flavour based on the
        // Request-URI's scheme + `;transport=` parameter.
        //
        let effective_tls_mode = config.effective_tls_mode();
        let enable_tls = effective_tls_mode != SipTlsMode::Disabled;
        if config.tls_cert_path.is_some() ^ config.tls_key_path.is_some() {
            tracing::warn!(
                "rvoip-sip Config has tls_cert_path xor tls_key_path set; \
                 TLS listener roles require both"
            );
        }
        if matches!(
            effective_tls_mode,
            SipTlsMode::ServerOnly | SipTlsMode::ClientAndServer
        ) && (config.tls_cert_path.is_none() || config.tls_key_path.is_none())
        {
            return Err(SessionError::ConfigError(
                "SIP TLS listener modes require tls_cert_path and tls_key_path".to_string(),
            ));
        }
        if config.tls_client_cert_path.is_some() ^ config.tls_client_key_path.is_some() {
            return Err(SessionError::ConfigError(
                "TLS client certificate and key must be provided together".to_string(),
            ));
        }

        let tls_role = match effective_tls_mode {
            SipTlsMode::Disabled => {
                rvoip_sip_dialog::transaction::transport::TlsRole::ClientAndServer
            }
            SipTlsMode::ClientOnly => rvoip_sip_dialog::transaction::transport::TlsRole::ClientOnly,
            SipTlsMode::ServerOnly => rvoip_sip_dialog::transaction::transport::TlsRole::ServerOnly,
            SipTlsMode::ClientAndServer => {
                rvoip_sip_dialog::transaction::transport::TlsRole::ClientAndServer
            }
        };
        if matches!(effective_tls_mode, SipTlsMode::ClientOnly) {
            tracing::info!(
                "SIP TLS client-only mode enabled; no local endpoint certificate/key required"
            );
        }
        let transport_config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: true,
            enable_ws: false,
            enable_tls,
            tls_role,
            bind_addresses: vec![config.bind_addr],
            tls_bind_addresses: config.tls_bind_addr.into_iter().collect(),
            tls_cert_path: config
                .tls_cert_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            tls_key_path: config
                .tls_key_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            tls_client_cert_path: config
                .tls_client_cert_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            tls_client_key_path: config
                .tls_client_key_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            tls_server_client_auth: config.tls_server_client_auth.clone(),
            tls_extra_ca_path: config
                .tls_extra_ca_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            default_channel_capacity: config.sip_transport_channel_capacity,
            udp_recv_buffer_size: config.sip_udp_recv_buffer_size,
            udp_send_buffer_size: config.sip_udp_send_buffer_size,
            udp_parse_workers: config.sip_udp_parse_workers,
            udp_parse_queue_capacity: config.sip_udp_parse_queue_capacity,
            udp_parse_dispatch: config.sip_udp_parse_dispatch,
            transport_event_dispatch_workers: config.sip_transport_dispatch_workers,
            transport_event_dispatch_queue_capacity: config.sip_transport_dispatch_queue_capacity,
            // Default build: `Config::tls_insecure_skip_verify` is not
            // compiled, so we always pass `false`. Only the
            // `dev-insecure-tls` build surfaces the field.
            #[cfg(feature = "dev-insecure-tls")]
            tls_insecure_skip_verify: config.tls_insecure_skip_verify,
            #[cfg(not(feature = "dev-insecure-tls"))]
            tls_insecure_skip_verify: false,
        };

        let dialog_tls_local_address = config.tls_bind_addr.or_else(|| {
            if matches!(
                effective_tls_mode,
                SipTlsMode::ServerOnly | SipTlsMode::ClientAndServer
            ) {
                let mut tls_addr = config.bind_addr;
                if tls_addr.port() != 0 {
                    tls_addr.set_port(tls_addr.port().saturating_add(1));
                }
                Some(tls_addr)
            } else {
                None
            }
        });

        let (mut transport_manager, transport_event_rx) =
            TransportManager::new(transport_config).await.map_err(|e| {
                SessionError::InternalError(format!("Failed to create transport manager: {}", e))
            })?;

        if let Some(owner_id) = sip_trace_owner_id {
            // SIP_API_DESIGN_2 §12.4 — always install an effective redactor.
            // Missing configuration resolves to the production-safe default;
            // verbatim tracing requires an explicit PassthroughRedactor.
            // The transform runs at the trace boundary in
            // SipTraceRuntime::publish; the wire form is unaffected.
            let redactor = config
                .trace_redaction
                .clone()
                .unwrap_or_else(crate::api::trace_redactor::default_trace_redactor);
            let redactor_fn: Option<rvoip_sip_dialog::transaction::transport::TraceRedactorFn> =
                Some(Arc::new(move |raw: &str| -> String {
                    crate::api::trace_redactor::apply_message_redactor(redactor.as_ref(), raw)
                }));

            transport_manager.enable_sip_trace_with_redactor(
                owner_id,
                config.sip_trace.clone(),
                global_coordinator.clone(),
                redactor_fn,
            );
        }

        // Initialize the transport manager
        transport_manager.initialize().await.map_err(|e| {
            SessionError::InternalError(format!("Failed to initialize transport: {}", e))
        })?;

        // Create transaction manager using transport manager
        let (transaction_manager, event_rx) =
            TransactionManager::with_transport_manager_and_index_capacity_and_dispatch_and_authorizer_shared_with_compact_retention_capacity(
                transport_manager,
                transport_event_rx,
                Some(config.transaction_event_channel_capacity),
                Some(config.transaction_index_capacity_hint()),
                config.sip_transaction_dispatch_workers,
                config.sip_transaction_dispatch_queue_capacity,
                listener_auth_policy.into_authorizer(),
                config
                    .server_retained_lifecycle_capacity
                    .unwrap_or(262_144),
            )
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to create transaction manager: {}", e))
            })?;

        let mut transaction_manager = transaction_manager;
        transaction_manager.set_auto_100_trying(config.auto_100_trying);
        transaction_manager.set_transaction_command_channel_capacity(
            config
                .sip_transaction_command_channel_capacity
                .unwrap_or(Config::DEFAULT_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY),
        );
        if let Some(max_burst) = config.sip_transaction_dispatch_priority_burst_max {
            transaction_manager.set_transaction_dispatch_priority_burst_max(max_burst);
        }
        if let Some(max_due_per_tick) = config.sip_invite_2xx_retransmit_max_due_per_tick {
            transaction_manager.set_invite_2xx_retransmit_max_due_per_tick(max_due_per_tick);
        }
        let transaction_manager = Arc::new(transaction_manager);

        // Create dialog config - use hybrid mode to support both incoming and outgoing calls
        let dialog_config = DialogManagerConfig::hybrid(config.bind_addr)
            .with_from_uri(&config.local_uri)
            .with_auto_options()
            .with_100rel(config.use_100rel)
            .with_session_timer(config.session_timer_secs)
            .with_min_se(config.session_timer_min_se)
            .with_dialog_config(|mut dialog| {
                dialog.advertised_local_address = config.sip_advertised_addr;
                dialog.local_contact_uri = config.contact_uri.clone();
                dialog.tls_local_address = dialog_tls_local_address;
                dialog.tls_advertised_local_address = config.tls_advertised_addr;
                dialog.max_dialogs = Some(config.dialog_index_capacity_hint());
                dialog.event_dispatch_workers = config.sip_dialog_dispatch_workers;
                dialog.event_dispatch_queue_capacity = config.sip_dialog_dispatch_queue_capacity;
                dialog
            })
            .build();

        // Create dialog API with global event coordination AND transaction events
        let dialog_api = Arc::new(
            UnifiedDialogApi::with_shared_global_events_and_coordinator(
                transaction_manager,
                event_rx,
                dialog_config,
                global_coordinator.clone(),
            )
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to create dialog API: {}", e))
            })?,
        );

        dialog_api.start().await.map_err(|e| {
            SessionError::InternalError(format!("Failed to start dialog API: {}", e))
        })?;

        Ok(dialog_api)
    }

    async fn create_media_controller(
        config: &Config,
        global_coordinator: Arc<GlobalEventCoordinator>,
        symmetric_rtp_policy: SymmetricRtpPolicy,
    ) -> Result<Arc<rvoip_media_core::relay::controller::MediaSessionController>> {
        use rvoip_media_core::relay::controller::MediaSessionController;

        let mut media_controller_config = config.media_session_controller_config.clone();
        media_controller_config.capacity_hint = config
            .media_session_capacity
            .or(config.server_call_capacity)
            .unwrap_or(media_controller_config.capacity_hint);
        media_controller_config.rtp_session_buffer_config = config.rtp_session_buffer_config;
        media_controller_config.rtp_transport_buffer_config = config.rtp_transport_buffer_config;

        // Create media controller with port range and SIP-exposed media/RTP tuning.
        let controller = Arc::new(
            MediaSessionController::with_port_range_and_config(
                config.media_port_start,
                config.media_port_end,
                media_controller_config,
            )
            .with_symmetric_rtp_policy(symmetric_rtp_policy),
        );

        // Create and set up the event hub
        let event_hub =
            rvoip_media_core::events::MediaEventHub::new(global_coordinator, controller.clone())
                .await
                .map_err(|e| {
                    SessionError::InternalError(format!("Failed to create media event hub: {}", e))
                })?;

        // Set the event hub on the media controller
        controller.set_event_hub(event_hub).await;

        Ok(controller)
    }
}

/// SIP_API_DESIGN_2 §7.1 — builder → state-machine dispatch helpers.
///
/// These two thin wrappers form the canonical builder send path. Builders
/// call [`UnifiedCoordinator::stage_outbound_options`] to write the matching
/// `pending_<method>_options` slot under the §7.3 invariant #5 conflict
/// guard, then call [`UnifiedCoordinator::dispatch_outbound`] with the
/// matching `EventType::SendOutbound<METHOD>` to drive the state table.
/// The action handler reads from the stash; the final-response transition
/// (`ClearPending*Options`) drops it.
impl UnifiedCoordinator {
    /// Stage a pending-options snapshot on the session and check the
    /// single-in-flight conflict guard. Returns
    /// `Err(SessionError::Conflict { method })` when a prior `.send()`
    /// for the same method on the same session has not yet reached its
    /// final response. See
    /// [`StateMachine::stage_outbound_options`](crate::state_machine::executor::StateMachine::stage_outbound_options).
    pub async fn stage_outbound_options(
        &self,
        session_id: &SessionId,
        slot: crate::state_machine::executor::PendingOptionsSlot,
    ) -> Result<()> {
        self.helpers
            .state_machine
            .stage_outbound_options(session_id, slot)
            .await
            .map_err(|e| {
                if let Ok(typed) = e.downcast::<SessionError>() {
                    *typed
                } else {
                    SessionError::InternalError(
                        "stage_outbound_options: state-machine error".to_string(),
                    )
                }
            })
    }

    pub(crate) async fn stage_outbound_options_guarded(
        &self,
        session_id: &SessionId,
        slot: crate::state_machine::executor::PendingOptionsSlot,
    ) -> Result<crate::state_machine::executor::PendingOptionsStageGuard> {
        self.helpers
            .state_machine
            .stage_outbound_options_guarded(session_id, slot)
            .await
            .map_err(|error| {
                if let Ok(typed) = error.downcast::<SessionError>() {
                    *typed
                } else {
                    SessionError::InternalError(
                        "stage_outbound_options_guarded: state-machine error".to_string(),
                    )
                }
            })
    }

    /// Queue a state-machine event on the session's event queue and run
    /// the resulting transition. Thin wrapper over
    /// [`StateMachine::process_event`].
    pub async fn dispatch_outbound(
        &self,
        session_id: &SessionId,
        event: crate::state_table::EventType,
    ) -> Result<crate::state_machine::executor::ProcessEventResult> {
        // `SendOutboundNotify` has one legacy, stack-generated shape in
        // addition to the public builder path: an automatic NOTIFY with no
        // pre-staged options. Materialize that shape into an exact guarded
        // stage before dispatch so it receives the same cancellation,
        // fast-response, authentication-retry, and completion ownership as a
        // builder-generated NOTIFY. The action itself remains fail-closed for
        // any unguarded unstaged dispatch that bypasses this coordinator.
        if matches!(&event, crate::state_table::EventType::SendOutboundNotify) {
            let (has_pending_options, local_sdp) = self
                .helpers
                .state_machine
                .store
                .with_session(session_id, |session| {
                    (
                        session.pending_notify_options.is_some(),
                        session.local_sdp.clone(),
                    )
                })
                .map_err(|_| SessionError::SessionNotFound(session_id.to_string()))?;

            if !has_pending_options {
                let options = Arc::new(rvoip_sip_dialog::api::unified::NotifyRequestOptions {
                    event: "presence".to_string(),
                    subscription_state: String::new(),
                    content_type: None,
                    body: local_sdp.map(bytes::Bytes::from),
                    subscription_id: None,
                    extra_headers: self.dialog_adapter.auto_emit_extra_headers.clone(),
                });
                let staging = self
                    .stage_outbound_options_guarded(
                        session_id,
                        crate::state_machine::executor::PendingOptionsSlot::Notify(options),
                    )
                    .await?;
                let result = self
                    .dispatch_outbound_guarded(session_id, event, &staging)
                    .await?;
                staging.confirm_consumed().await?;
                return Ok(result);
            }
        }

        let state_machine = Arc::clone(&self.helpers.state_machine);
        let task_session_id = session_id.clone();
        let task = AbortOutboundDispatchTaskOnDrop::new(tokio::spawn(async move {
            state_machine.process_event(&task_session_id, event).await
        }));
        task.join()
            .await
            .map_err(|_| SessionError::InternalError(OUTBOUND_DISPATCH_JOIN_FAILURE.to_string()))?
            .map_err(|e| SessionError::InternalError(format!("dispatch_outbound: {}", e)))
    }

    /// Dispatch an event that owns a guarded exact builder stage.
    ///
    /// Cancellation before the state-machine atomically claims the staged Arc
    /// aborts the task. Cancellation after claim detaches it so the request's
    /// first transport write and tracker activation cannot be split.
    pub(crate) async fn dispatch_outbound_guarded(
        &self,
        session_id: &SessionId,
        event: crate::state_table::EventType,
        staging: &crate::state_machine::executor::PendingOptionsStageGuard,
    ) -> Result<crate::state_machine::executor::ProcessEventResult> {
        let state_machine = Arc::clone(&self.helpers.state_machine);
        let task_session_id = session_id.clone();
        let stage_claim = staging.dispatch_claim();
        let task_claim = Arc::clone(&stage_claim);
        let task = AbortOutboundDispatchTaskOnDrop::with_stage_claim(
            tokio::spawn(async move {
                state_machine
                    .process_event_with_stage_claim(&task_session_id, event, task_claim)
                    .await
            }),
            stage_claim,
        );
        task.join()
            .await
            .map_err(|_| SessionError::InternalError(OUTBOUND_DISPATCH_JOIN_FAILURE.to_string()))?
            .map_err(|e| SessionError::InternalError(format!("dispatch_outbound: {}", e)))
    }

    /// Crate-internal accessor: read the current `SessionState` snapshot for
    /// the given session id. Used by refresh-style builders that need to
    /// reuse registration / dialog identifiers from the original send.
    pub(crate) async fn session_state(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::session_store::SessionState> {
        self.helpers
            .state_machine
            .store
            .get_session(session_id)
            .await
            .map_err(|_| SessionError::SessionNotFound(session_id.to_string()))
    }

    /// Crate-internal: write back a modified `SessionState`. Used by
    /// response builders to stash extras (`Retry-After`, `Warning`,
    /// `WWW-Authenticate`, …) on the session before firing the
    /// state-machine event that consumes them.
    pub(crate) async fn update_session_state(
        &self,
        session: crate::session_store::SessionState,
    ) -> Result<()> {
        self.helpers
            .state_machine
            .store
            .update_session(session)
            .await
            .map_err(|e| SessionError::InternalError(format!("update_session: {}", e)))
    }
}

/// Simple helper to create a session and make a call
impl UnifiedCoordinator {
    /// Quick method to create a UAC session and make a call
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// let call_id = coordinator
    ///     .quick_call("sip:alice@127.0.0.1:5060", "sip:bob@127.0.0.1:5070")
    ///     .await?;
    /// # let _ = call_id;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn quick_call(self: &Arc<Self>, from: &str, to: &str) -> Result<SessionId> {
        self.invite(Some(from.to_string()), to.to_string())
            .send()
            .await
    }
}

/// Registration API
impl UnifiedCoordinator {
    /// Internal REGISTER dispatch used by
    /// [`RegisterBuilder`](crate::api::send::RegisterBuilder).
    ///
    /// When `extra_headers` is non-empty we follow SIP_API_DESIGN_2 §10 #19
    /// and stash a `RegisterRequestOptions` on the session *before* the
    /// `StartRegistration` event fires, so `execute_register_action`
    /// (`state_machine/actions.rs`) reads the slice on the very first
    /// dispatch (not just on the 401-retry).
    ///
    /// When `extra_headers` is empty we skip the stash entirely. Stashing
    /// here would occupy `pending_register_options`; the slot is only
    /// cleared once the first REGISTER reaches a final response, so a
    /// caller that fires a `RegisterRefreshBuilder::send` before that
    /// would race the stash check in
    /// [`stage_outbound_options`](crate::state_machine::executor::StateMachineExecutor::stage_outbound_options)
    /// and get back `SessionError::Conflict { method: Register }`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn register_with_extras(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        username: &str,
        password: &str,
        expires: u32,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<RegistrationHandle> {
        let session_id = SessionId::new();
        self.helpers
            .create_session(
                session_id.clone(),
                from_uri.to_string(),
                registrar_uri.to_string(),
                crate::state_table::types::Role::UAC,
            )
            .await?;

        let credentials = crate::types::Credentials::new(username, password);

        // If the application didn't configure UAC auth, adopt these REGISTER
        // credentials as the default for challenged in-account requests
        // (INVITE/re-INVITE/BYE/REFER) so "register, then call" authenticates
        // out of the box. Explicit Config.credentials/Config.auth always win
        // (see `config_credentials`). Locked briefly and synchronously.
        if self.config.credentials.is_none() && self.config.auth.is_none() {
            if let Ok(mut slot) = self.registered_credentials.lock() {
                *slot = Some(credentials.clone());
            }
        }

        let session_store = &self.helpers.state_machine.store;
        session_store
            .update_session_with(&session_id, |session| {
                session.credentials = Some(credentials);
                session.registrar_uri = Some(registrar_uri.to_string());
                session.registration_contact = Some(contact_uri.to_string());
                session.registration_expires = Some(expires);

                if !extra_headers.is_empty() {
                    session.pending_register_options = Some(std::sync::Arc::new(
                        rvoip_sip_dialog::api::unified::RegisterRequestOptions {
                            registrar_uri: registrar_uri.to_string(),
                            aor_uri: from_uri.to_string(),
                            contact_uri: contact_uri.to_string(),
                            expires,
                            authorization: None,
                            proxy_authorization: None,
                            call_id: None,
                            cseq: None,
                            outbound_contact: None,
                            outbound_proxy_uri: None,
                            extra_headers,
                            refresh: false,
                        },
                    ));
                }
            })
            .await?;

        let _ = self
            .helpers
            .state_machine
            .process_event(
                &session_id,
                crate::state_table::types::EventType::StartRegistration,
            )
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to trigger registration: {}", e))
            })?;

        Ok(RegistrationHandle { session_id })
    }

    /// Unregister from the SIP server.
    ///
    /// Sends REGISTER with `Expires: 0` to remove the binding and aborts any
    /// pending automatic refresh task when the registrar confirms success.
    /// This method returns after the state machine accepts the request. Use
    /// [`unregister_and_wait`](Self::unregister_and_wait) when the caller must
    /// wait for `UnregistrationSuccess` or `UnregistrationFailed`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// coordinator.unregister(&handle).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn unregister(&self, handle: &RegistrationHandle) -> Result<()> {
        // Trigger unregistration via state machine
        let result = self
            .helpers
            .state_machine
            .process_event(
                &handle.session_id,
                crate::state_table::types::EventType::StartUnregistration,
            )
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to trigger unregistration: {}", e))
            })?;
        if result.transition.is_none() {
            return Err(SessionError::InvalidTransition(format!(
                "Cannot unregister session {} from state {:?}",
                handle.session_id.0, result.old_state
            )));
        }
        if !result
            .actions_executed
            .iter()
            .any(|action| matches!(action, Action::SendUnREGISTER))
        {
            return Err(SessionError::InternalError(format!(
                "Unregistration transition for session {} did not send REGISTER Expires: 0",
                handle.session_id.0
            )));
        }
        Ok(())
    }

    /// Refresh registration before it expires.
    ///
    /// Sends a new REGISTER request using the stored registration expiry and
    /// registration Call-ID. Successful refresh responses replace the stored
    /// accepted expiry and next automatic refresh time.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// coordinator.refresh_registration(&handle).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn refresh_registration(&self, handle: &RegistrationHandle) -> Result<()> {
        // Trigger refresh via state machine
        let _result = self
            .helpers
            .state_machine
            .process_event(
                &handle.session_id,
                crate::state_table::types::EventType::RefreshRegistration,
            )
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to trigger refresh: {}", e))
            })?;
        Ok(())
    }

    /// SIP_API_DESIGN_2 §3.3 — Begin a manual REGISTER refresh on the
    /// given registration handle. Returns a `RegisterRefreshBuilder`
    /// that supports `.with_expires(n)` and ships via `.send().await`.
    pub fn refresh(
        self: &Arc<Self>,
        handle: &RegistrationHandle,
    ) -> crate::api::send::RegisterRefreshBuilder {
        crate::api::send::RegisterRefreshBuilder::new(self.clone(), handle.clone())
    }

    /// Return whether the registration is currently marked registered.
    ///
    /// This is a coarse boolean for simple clients. Use
    /// [`registration_info`](Self::registration_info) for status, accepted
    /// expiry, next refresh timing, failure metadata, Service-Route, GRUU, and
    /// outbound-flow information.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// if coordinator.is_registered(&handle).await? {
    ///     println!("registration is active");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_registered(&self, handle: &RegistrationHandle) -> Result<bool> {
        let (is_registered, retry_count) = self
            .helpers
            .state_machine
            .store
            .with_session(&handle.session_id, |session| {
                (session.is_registered, session.registration_retry_count)
            })?;
        tracing::info!(
            "🔍 Checking registration for session {}: is_registered={}, retry_count={}",
            handle.session_id.0,
            is_registered,
            retry_count
        );
        Ok(is_registered)
    }

    /// Return detailed registration lifecycle information for a handle.
    ///
    /// `accepted_expires_secs`, `registered_at`, and `next_refresh_at` are
    /// populated from successful REGISTER responses. `service_route`,
    /// `pub_gruu`, and `temp_gruu` are populated when supplied by the
    /// registrar. Failure and unregister paths clear refresh metadata and keep
    /// a stable status snapshot for diagnostics.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// let info = coordinator.registration_info(&handle).await?;
    /// println!("status: {:?}", info.status);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn registration_info(&self, handle: &RegistrationHandle) -> Result<RegistrationInfo> {
        let snapshot = self
            .helpers
            .state_machine
            .store
            .get_session_snapshot(&handle.session_id)
            .await?;
        let (
            status,
            next_refresh_in,
            last_failure,
            aor,
            stored_service_route,
            stored_pub_gruu,
            stored_temp_gruu,
            registrar,
            contact,
            expires_secs,
            retry_count,
            accepted_expires_secs,
            registered_at,
            next_refresh_at,
        ) = {
            let session = snapshot.state();
            let status = if session.is_registered {
                RegistrationStatus::Registered
            } else {
                match session.call_state {
                    CallState::Registering => RegistrationStatus::Registering,
                    CallState::Unregistering => RegistrationStatus::Unregistering,
                    _ if session.registration_last_failure.is_some() => RegistrationStatus::Failed,
                    _ if session.registration_retry_count > 0 => RegistrationStatus::Failed,
                    _ => RegistrationStatus::Unregistered,
                }
            };
            let next_refresh_in = session
                .registration_next_refresh_at
                .map(|when| when.saturating_duration_since(Instant::now()));
            let last_failure = if let Some(reason) = session.registration_last_failure.clone() {
                Some(reason)
            } else if matches!(status, RegistrationStatus::Failed) {
                Some(format!(
                    "registration failed after {} retry attempt(s)",
                    session.registration_retry_count
                ))
            } else {
                None
            };
            (
                status,
                next_refresh_in,
                last_failure,
                session.local_uri.clone(),
                session.registration_service_route.clone(),
                session.registration_pub_gruu.clone(),
                session.registration_temp_gruu.clone(),
                session.registrar_uri.clone(),
                session.registration_contact.clone(),
                session.registration_expires,
                session.registration_retry_count,
                session.registration_accepted_expires,
                session.registration_registered_at,
                session.registration_next_refresh_at,
            )
        };
        drop(snapshot);

        let (dialog_service_route, dialog_pub_gruu, dialog_temp_gruu, outbound_flow_active) =
            if let Some(aor) = aor.as_deref() {
                let dialog_service_route = self
                    .dialog_adapter
                    .dialog_api
                    .service_route_for_aor(aor)
                    .await
                    .map(|uris| uris.into_iter().map(|uri| uri.to_string()).collect());
                let gruu = self.dialog_adapter.dialog_api.gruu_for_aor(aor).await;
                let outbound_flow_active = self
                    .dialog_adapter
                    .dialog_api
                    .outbound_flow_active_for_aor(aor);
                (
                    dialog_service_route,
                    gruu.as_ref().and_then(|params| params.pub_gruu.clone()),
                    gruu.and_then(|params| params.temp_gruu),
                    outbound_flow_active,
                )
            } else {
                (None, None, None, false)
            };
        let service_route = stored_service_route.or(dialog_service_route);
        let pub_gruu = stored_pub_gruu.or(dialog_pub_gruu);
        let temp_gruu = stored_temp_gruu.or(dialog_temp_gruu);

        Ok(RegistrationInfo {
            session_id: handle.session_id.clone(),
            status,
            registrar,
            contact,
            expires_secs,
            next_refresh_in,
            retry_count,
            last_failure,
            accepted_expires_secs,
            registered_at,
            next_refresh_at,
            service_route,
            pub_gruu,
            temp_gruu,
            outbound_flow_active,
        })
    }

    /// Unregister and wait for the matching registration lifecycle event.
    ///
    /// This subscribes to the coordinator event stream before sending
    /// unregister, then returns after `UnregistrationSuccess` or converts
    /// `UnregistrationFailed` into an error. Registration events are global
    /// coordinator events, not per-registration handle streams.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// coordinator
    ///     .unregister_and_wait(&handle, Some(std::time::Duration::from_secs(3)))
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn unregister_and_wait(
        &self,
        handle: &RegistrationHandle,
        timeout: Option<std::time::Duration>,
    ) -> Result<()> {
        let registrar = self
            .registration_info(handle)
            .await?
            .registrar
            .unwrap_or_default();
        let mut events = self.events().await?;
        self.unregister(handle).await?;

        let fut = async {
            loop {
                match events.next().await {
                    Some(crate::api::events::Event::UnregistrationSuccess { registrar: r })
                        if registrar.is_empty() || r == registrar =>
                    {
                        return Ok(());
                    }
                    Some(crate::api::events::Event::UnregistrationFailed {
                        registrar: r,
                        reason,
                    }) if registrar.is_empty() || r == registrar => {
                        return Err(SessionError::Other(format!(
                            "Unregistration failed for {}: {}",
                            r, reason
                        )));
                    }
                    Some(_) => {}
                    None => {
                        return Err(SessionError::Other(
                            "Event channel closed while waiting for unregister".to_string(),
                        ));
                    }
                }
            }
        };

        match timeout {
            Some(duration) => tokio::time::timeout(duration, fut)
                .await
                .map_err(|_| SessionError::Timeout("unregister_and_wait timed out".to_string()))?,
            None => fut.await,
        }
    }
}

/// Handle for managing a registration.
///
/// Registration lifecycle events are emitted through
/// [`UnifiedCoordinator::events`] and [`UnifiedCoordinator::events_for_session`].
/// This handle deliberately does not expose a separate event stream today,
/// because doing so cleanly would require a per-registration event bus split.
#[derive(Clone)]
pub struct RegistrationHandle {
    /// Session id backing this registration lifecycle.
    pub session_id: SessionId,
}

impl std::fmt::Debug for RegistrationHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegistrationHandle")
            .field("session_id_present", &true)
            .finish()
    }
}

/// Coarse registration lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationStatus {
    /// REGISTER has been sent and the registrar response is still pending.
    Registering,
    /// Registrar accepted the binding and the contact is currently active.
    Registered,
    /// REGISTER with `Expires: 0` has been sent and the registrar response is pending.
    Unregistering,
    /// No active binding is known for this registration handle.
    Unregistered,
    /// The most recent registration or refresh attempt failed.
    Failed,
}

/// Query result for a registration handle.
///
/// This is a snapshot of the current client-side registration lifecycle. It
/// combines rvoip-sip state with metadata learned from rvoip-sip-dialog REGISTER
/// responses.
#[derive(Clone)]
pub struct RegistrationInfo {
    /// Session id backing this registration lifecycle.
    pub session_id: SessionId,
    /// Coarse lifecycle status.
    pub status: RegistrationStatus,
    /// Registrar URI originally used for REGISTER.
    pub registrar: Option<String>,
    /// Contact URI currently associated with the registration.
    pub contact: Option<String>,
    /// Last expiry value rvoip-sip will request on refresh.
    pub expires_secs: Option<u32>,
    /// Duration until the currently scheduled automatic refresh.
    pub next_refresh_in: Option<Duration>,
    /// Number of retry attempts used by the current/last registration flow.
    pub retry_count: u32,
    /// Last failure summary, if the lifecycle is failed.
    pub last_failure: Option<String>,
    /// Expiry accepted by the registrar in the most recent successful 2xx.
    pub accepted_expires_secs: Option<u32>,
    /// Local time when the most recent successful registration completed.
    pub registered_at: Option<Instant>,
    /// Local time when automatic refresh is scheduled.
    pub next_refresh_at: Option<Instant>,
    /// Registrar-provided Service-Route URIs, if supplied.
    pub service_route: Option<Vec<String>>,
    /// Registrar-assigned public GRUU, if supplied.
    pub pub_gruu: Option<String>,
    /// Registrar-assigned temporary GRUU, if supplied.
    pub temp_gruu: Option<String>,
    /// Whether rvoip-sip-dialog currently has an RFC 5626 outbound flow monitor.
    pub outbound_flow_active: bool,
}

impl std::fmt::Debug for RegistrationInfo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegistrationInfo")
            .field("session_id_present", &true)
            .field("status", &self.status)
            .field("registrar_configured", &self.registrar.is_some())
            .field("contact_configured", &self.contact.is_some())
            .field("expires_secs", &self.expires_secs)
            .field("next_refresh_scheduled", &self.next_refresh_in.is_some())
            .field("retry_count", &self.retry_count)
            .field("last_failure_present", &self.last_failure.is_some())
            .field("accepted_expires_secs", &self.accepted_expires_secs)
            .field("registered", &self.registered_at.is_some())
            .field("next_refresh_at_present", &self.next_refresh_at.is_some())
            .field(
                "service_route_count",
                &self.service_route.as_ref().map_or(0, Vec::len),
            )
            .field("public_gruu_present", &self.pub_gruu.is_some())
            .field("temporary_gruu_present", &self.temp_gruu.is_some())
            .field("outbound_flow_active", &self.outbound_flow_active)
            .finish()
    }
}

/// Configuration for SIP registration.
///
/// Use [`Registration::new()`] for the common case where `from_uri` and
/// `contact_uri` are derived from the peer's [`Config`].
///
/// # Example
///
/// ```
/// use rvoip_sip::Registration;
///
/// let reg = Registration::new("sip:registrar.example.com", "alice", "secret123")
///     .expires(1800);
/// ```
#[derive(Clone)]
pub struct Registration {
    /// SIP URI of the registrar server (e.g. `sip:registrar.example.com`)
    pub registrar: String,
    /// Username for digest authentication
    pub username: String,
    /// Password for digest authentication
    pub password: String,
    /// Registration expiry in seconds (default: 3600)
    pub expires: u32,
    /// Override the From URI (defaults to the peer's local_uri)
    pub from_uri: Option<String>,
    /// Override the Contact URI (defaults to the peer's local_uri)
    pub contact_uri: Option<String>,
}

impl std::fmt::Debug for Registration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Registration")
            .field("registrar_configured", &!self.registrar.is_empty())
            .field("username_configured", &!self.username.is_empty())
            .field("password_configured", &!self.password.is_empty())
            .field("expires", &self.expires)
            .field("from_uri_configured", &self.from_uri.is_some())
            .field("contact_uri_configured", &self.contact_uri.is_some())
            .finish()
    }
}

impl Registration {
    /// Create a registration with the minimum required fields.
    ///
    /// `from_uri` and `contact_uri` will be derived from the peer's config.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip::Registration;
    ///
    /// let reg = Registration::new("sip:registrar.example.com", "alice", "secret");
    /// assert_eq!(reg.expires, 3600);
    /// ```
    pub fn new(
        registrar: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            registrar: registrar.into(),
            username: username.into(),
            password: password.into(),
            expires: 3600,
            from_uri: None,
            contact_uri: None,
        }
    }

    /// Set the registration expiry in seconds (default: 3600).
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip::Registration;
    ///
    /// let reg = Registration::new("sip:registrar.example.com", "alice", "secret")
    ///     .expires(600);
    /// assert_eq!(reg.expires, 600);
    /// ```
    pub fn expires(mut self, secs: u32) -> Self {
        self.expires = secs;
        self
    }

    /// Override the From URI (defaults to the peer's local URI).
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip::Registration;
    ///
    /// let reg = Registration::new("sip:registrar.example.com", "alice", "secret")
    ///     .from_uri("sip:alice@example.com");
    /// assert_eq!(reg.from_uri.as_deref(), Some("sip:alice@example.com"));
    /// ```
    pub fn from_uri(mut self, uri: impl Into<String>) -> Self {
        self.from_uri = Some(uri.into());
        self
    }

    /// Override the Contact URI (defaults to the peer's local URI).
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip::Registration;
    ///
    /// let reg = Registration::new("sip:registrar.example.com", "alice", "secret")
    ///     .contact_uri("sip:alice@192.168.1.50:5060");
    /// assert_eq!(reg.contact_uri.as_deref(), Some("sip:alice@192.168.1.50:5060"));
    /// ```
    pub fn contact_uri(mut self, uri: impl Into<String>) -> Self {
        self.contact_uri = Some(uri.into());
        self
    }
}

/// Sprint 3 A6 — best-effort STUN probe for the RTP-side public
/// mapping.
///
/// **Caveat.** The probe binds a fresh ephemeral UDP socket on the
/// configured `local_ip` and asks the STUN server what mapping it
/// sees. For typical cone NATs (most consumer routers, AWS / GCP
/// NAT gateways) the mapping is keyed by source IP only, so the
/// discovered address matches what the actual RTP path will see
/// later. For symmetric NATs the mapping is per-(source IP, source
/// port) and the result will be wrong — those deployments need ICE
/// (Sprint 4 D3). For Sprint 3 the simple shape is the right
/// trade-off; a deployment that breaks here can fall back to
/// `Config::media_public_addr` (static override).
async fn run_stun_probe(adapter: Arc<MediaAdapter>, stun_target: &str) -> Result<()> {
    use std::sync::Arc as StdArc;
    use tokio::net::UdpSocket as TokioUdpSocket;

    // Normalise "host" → "host:3478"; "host:port" passes through.
    let target_str = if stun_target.contains(':') {
        stun_target.to_string()
    } else {
        format!("{}:3478", stun_target)
    };

    // Resolve via tokio's DNS — STUN servers are typically fronted by
    // SRV in production but the public ones (Google, Cloudflare) all
    // expose plain A records.
    let server_addr = tokio::net::lookup_host(&target_str)
        .await
        .map_err(|e| {
            SessionError::ConfigError(format!("STUN resolve '{}' failed: {}", target_str, e))
        })?
        .next()
        .ok_or_else(|| {
            SessionError::ConfigError(format!("STUN '{}' resolved to nothing", target_str))
        })?;

    // Bind a probe socket on the same interface as the SIP/media
    // bind. Random ephemeral port; the cone-NAT-mapping caveat above
    // applies.
    let bind_local = std::net::SocketAddr::new(adapter.local_ip(), 0);
    let probe_sock = TokioUdpSocket::bind(bind_local).await.map_err(|e| {
        SessionError::ConfigError(format!("STUN probe bind {} failed: {}", bind_local, e))
    })?;
    let probe_sock = StdArc::new(probe_sock);

    let client = rvoip_rtp_core::network::stun::StunClient::new(probe_sock, server_addr);
    let discovered = client
        .discover()
        .await
        .map_err(|e| SessionError::ConfigError(format!("STUN probe failed: {}", e)))?;

    tracing::info!(
        "RTP public addr: {} (STUN-discovered via {})",
        discovered,
        target_str
    );
    adapter.set_public_rtp_addr(Some(discovered));
    Ok(())
}
