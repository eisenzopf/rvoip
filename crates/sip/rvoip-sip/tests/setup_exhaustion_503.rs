//! An initial INVITE that cannot get media resources must end with one 503
//! carrying `Retry-After`, never with silence, and the UAS must serve new
//! calls again as soon as resources return.
//!
//! A media allocation failure inside `IncomingCall` used to return a handler
//! error, and dialog-core (by design) authors no fallback after one, so the
//! caller timed out while the server transaction and dialog stayed
//! allocated. Under sustained load at the limit that wedged the UAS.
//!
//! Scenarios (one process, Bob is the UAS, Alice the caller):
//! 1. port admission: Bob's RTP range holds two rtcp-mux calls and his
//!    session limit is higher, so the third INVITE is shed with 503 before
//!    any session exists; after one hangup a new call is answered again;
//! 2. setup-failure backstop: an outside socket occupies Bob's only RTP port.
//!    The allocator still counts it free, so admission passes and the real
//!    bind fails inside media setup; that INVITE must still get 503 and
//!    leave no session. Once the socket closes and the bind quarantine
//!    expires, the next call is answered;
//! 3. separate RTCP ports: with `rtcp_non_mux`, a non-mux offer needs an even
//!    port and the one above it. Once a multiplexed call splits the last
//!    pair, a non-mux INVITE is shed by admission while a mux INVITE still
//!    gets the remaining port.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::events::{CallId, Event};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::cleanup_diag::session_cleanup_counters;
use rvoip_sip::SipTraceConfig;

const WAIT: Duration = Duration::from_secs(10);
const RETRY_AFTER_SECS: u32 = 2;

/// The scenarios read process-wide counters, so they run one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct AcceptAll;

#[async_trait::async_trait]
impl CallHandler for AcceptAll {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
        CallHandlerDecision::Accept
    }
}

fn peer_config(name: &str, port: u16, media_start: u16, media_end: u16) -> Config {
    let mut config = Config::local(name, port);
    config.media_port_start = media_start;
    config.media_port_end = media_end;
    config
}

fn traced(config: Config) -> Config {
    let mut config = config;
    config.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: false,
        ..SipTraceConfig::default()
    };
    config.trace_passthrough_for_development()
}

#[derive(Default)]
struct Observed {
    answered: Vec<CallId>,
    failed: Vec<(CallId, u16)>,
    /// Raw inbound 503 responses to INVITE seen by the caller.
    invite_503s: Vec<String>,
}

struct Caller {
    alice: Arc<UnifiedCoordinator>,
    observed: Arc<Mutex<Observed>>,
    collector: tokio::task::JoinHandle<()>,
    from: Option<String>,
    target: String,
}

impl Caller {
    async fn start(alice_port: u16, alice_media: u16, bob_port: u16, offer_rtcp_mux: bool) -> Self {
        let mut config = traced(peer_config(
            "setup-exhaustion-alice",
            alice_port,
            alice_media,
            alice_media + 99,
        ));
        config.offer_rtcp_mux = offer_rtcp_mux;
        let alice = UnifiedCoordinator::new(config)
            .await
            .expect("alice UnifiedCoordinator::new");
        let observed = Arc::new(Mutex::new(Observed::default()));
        let mut events = alice.events().await.expect("alice events");
        let sink = Arc::clone(&observed);
        let collector = tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let mut observed = sink.lock().unwrap();
                match event {
                    Event::CallAnswered { call_id, .. } => observed.answered.push(call_id),
                    Event::CallFailed {
                        call_id,
                        status_code,
                        ..
                    } => observed.failed.push((call_id, status_code)),
                    Event::SipTrace(trace)
                        if format!("{:?}", trace.direction) == "Inbound"
                            && trace.start_line.starts_with("SIP/2.0 503")
                            && trace.raw_message.contains("INVITE") =>
                    {
                        observed.invite_503s.push(trace.raw_message.clone());
                    }
                    _ => {}
                }
            }
        });
        tokio::time::sleep(Duration::from_millis(250)).await;
        Self {
            alice,
            observed,
            collector,
            from: Some(format!("sip:alice@127.0.0.1:{alice_port}")),
            target: format!("sip:bob@127.0.0.1:{bob_port}"),
        }
    }

    async fn invite(&self) -> CallId {
        self.alice
            .invite(self.from.clone(), self.target.clone())
            .send()
            .await
            .expect("invite")
    }

    /// Wait for the call's first outcome: `Ok(())` answered, `Err(status)`.
    async fn outcome(&self, call: &CallId) -> Result<(), u16> {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            {
                let observed = self.observed.lock().unwrap();
                if observed.answered.contains(call) {
                    return Ok(());
                }
                if let Some((_, status)) = observed.failed.iter().find(|(id, _)| id == call) {
                    return Err(*status);
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "call {call:?} got no final response"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn assert_last_503_has_retry_after(&self) {
        let raw = self
            .observed
            .lock()
            .unwrap()
            .invite_503s
            .last()
            .cloned()
            .expect("an inbound 503 to INVITE was traced");
        let retry_after = raw.lines().any(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                name.trim().eq_ignore_ascii_case("Retry-After")
                    && value.trim() == RETRY_AFTER_SECS.to_string()
            })
        });
        assert!(retry_after, "503 without Retry-After: {raw}");
    }

    async fn hangup(&self, call: &CallId) {
        self.alice
            .session(call)
            .hangup_and_wait(Some(WAIT))
            .await
            .expect("hangup");
    }

    async fn shutdown(self) {
        let _ = self
            .alice
            .shutdown_gracefully(Some(Duration::from_secs(2)))
            .await;
        self.collector.abort();
    }
}

async fn wait_for_bob(bob: &Arc<UnifiedCoordinator>, sessions: usize, ports: usize) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let count = bob.list_sessions().await.len();
        let leased = bob.allocated_media_port_count().await;
        if count == sessions && leased == ports {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "bob kept {count} sessions and {leased} media ports, expected {sessions} and {ports}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

struct Bob {
    coordinator: Arc<UnifiedCoordinator>,
    shutdown: rvoip_sip::api::callback_peer::ShutdownHandle,
    task: tokio::task::JoinHandle<()>,
}

impl Bob {
    async fn start(port: u16, media_start: u16, media_end: u16, rtcp_non_mux: bool) -> Self {
        let config = peer_config("setup-exhaustion-bob", port, media_start, media_end)
            .with_server_call_admission_limit(10)
            .with_server_overload_retry_after_secs(RETRY_AFTER_SECS)
            .with_rtcp_non_mux(rtcp_non_mux);
        let bob = CallbackPeer::new(AcceptAll, config)
            .await
            .expect("bob CallbackPeer::new");
        let coordinator = Arc::clone(bob.coordinator());
        let shutdown = bob.shutdown_handle();
        let task = tokio::spawn(async move {
            let _ = bob.run().await;
        });
        Self {
            coordinator,
            shutdown,
            task,
        }
    }

    async fn stop(self) {
        self.shutdown.shutdown();
        let _ = tokio::time::timeout(Duration::from_secs(3), self.task).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_rtp_range_sheds_503_with_retry_after_and_recovers() {
    let _serial = SERIAL.lock().await;
    let bob = Bob::start(18_880, 29_880, 29_881, false).await;
    let caller = Caller::start(18_881, 36_200, 18_880, true).await;
    let before = session_cleanup_counters();

    let first = caller.invite().await;
    assert_eq!(caller.outcome(&first).await, Ok(()));
    let second = caller.invite().await;
    assert_eq!(caller.outcome(&second).await, Ok(()));

    // Both RTP ports are leased; the session limit (10) is not the bound.
    let shed = caller.invite().await;
    assert_eq!(caller.outcome(&shed).await, Err(503));
    caller.assert_last_503_has_retry_after();
    let after = session_cleanup_counters();
    assert_eq!(
        after.media_port_admission_rejects,
        before.media_port_admission_rejects + 1
    );
    assert_eq!(after.setup_failure_rejects, before.setup_failure_rejects);
    wait_for_bob(&bob.coordinator, 2, 2).await;

    // Recovery: one hangup frees one slot for the next call.
    caller.hangup(&first).await;
    wait_for_bob(&bob.coordinator, 1, 1).await;
    let after_hangup = caller.invite().await;
    assert_eq!(caller.outcome(&after_hangup).await, Ok(()));

    caller.hangup(&second).await;
    caller.hangup(&after_hangup).await;
    wait_for_bob(&bob.coordinator, 0, 0).await;
    bob.stop().await;
    caller.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn media_setup_failure_answers_503_with_retry_after_and_recovers() {
    let _serial = SERIAL.lock().await;
    // Occupy Bob's only RTP port from outside his allocator.
    let squatter = std::net::UdpSocket::bind(("0.0.0.0", 29_890)).expect("hold Bob's RTP port");
    let bob = Bob::start(18_890, 29_890, 29_890, false).await;
    let caller = Caller::start(18_891, 36_300, 18_890, true).await;
    let before = session_cleanup_counters();

    let failed = caller.invite().await;
    assert_eq!(caller.outcome(&failed).await, Err(503));
    caller.assert_last_503_has_retry_after();
    let after = session_cleanup_counters();
    assert_eq!(
        after.setup_failure_rejects,
        before.setup_failure_rejects + 1,
        "the 503 must come from the setup-failure path, not port admission"
    );
    assert_eq!(
        after.media_port_admission_rejects,
        before.media_port_admission_rejects
    );
    wait_for_bob(&bob.coordinator, 0, 0).await;

    // Resources return: the socket closes and the 1 s bind quarantine ends.
    drop(squatter);
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let recovered = caller.invite().await;
    assert_eq!(caller.outcome(&recovered).await, Ok(()));
    caller.hangup(&recovered).await;
    wait_for_bob(&bob.coordinator, 0, 0).await;

    bob.stop().await;
    caller.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_mux_offer_is_shed_when_no_rtp_rtcp_pair_is_free() {
    let _serial = SERIAL.lock().await;
    // Four ports: two RTP/RTCP pairs, or four multiplexed calls.
    let bob = Bob::start(18_870, 29_870, 29_873, true).await;
    let plain = Caller::start(18_871, 36_400, 18_870, false).await;
    let muxed = Caller::start(18_872, 36_500, 18_870, true).await;
    let before = session_cleanup_counters();

    // A non-mux offer reserves RTP and RTP + 1.
    let paired = plain.invite().await;
    assert_eq!(plain.outcome(&paired).await, Ok(()));
    wait_for_bob(&bob.coordinator, 1, 2).await;
    // A mux offer takes one port and splits the last pair.
    let single = muxed.invite().await;
    assert_eq!(muxed.outcome(&single).await, Ok(()));
    wait_for_bob(&bob.coordinator, 2, 3).await;

    // One port is free but no pair: the non-mux INVITE is shed by admission
    // instead of failing inside media setup.
    let shed = plain.invite().await;
    assert_eq!(plain.outcome(&shed).await, Err(503));
    plain.assert_last_503_has_retry_after();
    let after = session_cleanup_counters();
    assert_eq!(
        after.media_port_admission_rejects,
        before.media_port_admission_rejects + 1
    );
    assert_eq!(after.setup_failure_rejects, before.setup_failure_rejects);
    wait_for_bob(&bob.coordinator, 2, 3).await;

    // A mux INVITE still fits in the remaining port.
    let last = muxed.invite().await;
    assert_eq!(muxed.outcome(&last).await, Ok(()));
    wait_for_bob(&bob.coordinator, 3, 4).await;

    plain.hangup(&paired).await;
    muxed.hangup(&single).await;
    muxed.hangup(&last).await;
    wait_for_bob(&bob.coordinator, 0, 0).await;
    bob.stop().await;
    plain.shutdown().await;
    muxed.shutdown().await;
}
