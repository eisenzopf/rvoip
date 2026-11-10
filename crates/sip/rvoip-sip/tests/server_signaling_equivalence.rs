//! The opt-in inbound signaling fast paths
//! (`Config::server_call_admission_lock_free`,
//! `Config::inbound_session_history = false`,
//! `Config::session_inline_incoming_call_dispatch`) must not change what goes
//! on the wire or what the application observes.
//!
//! The same scenario runs twice, once with the defaults and once with all
//! three enabled, each time against fresh peers. Bob (the UAS) has a server
//! admission limit of one, so the admission decision runs for every INVITE:
//!
//! 1. full call: INVITE / 180 / 200 / ACK, re-INVITEs to hold and resume
//!    (each 200 / ACK), then BYE / 200 from the caller;
//! 2. handler rejection: INVITE / 180 / 486 / ACK;
//! 3. CANCEL while ringing: INVITE / 180, CANCEL / 200, 487 / ACK;
//! 4. admission rejection: a held call fills the limit and the next INVITE
//!    gets 503; the held call then ends with BYE;
//! 5. teardown: both peers return to zero sessions.
//!
//! The transcript is the caller's SIP trace (direction, start-line method or
//! status, CSeq number and method; retransmissions and 100 Trying removed)
//! plus the caller's application outcomes. Both arms must produce the same,
//! exactly expected, transcript.
//!
//! The fast-path arm also proves the hop it removes, by count: the dialog
//! layer records every dialog-to-session event a session shard dequeues, and
//! with inline dispatch no initial INVITE goes through a shard.

use std::sync::{Arc, Mutex};
use std::time::Duration;

#[path = "support/free_port.rs"]
mod free_port;

use free_port::{probe_udp_port, retry_address_in_use};

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::events::{CallId, Event};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{CallState, SipTraceConfig};

const WAIT: Duration = Duration::from_secs(8);

/// Initial INVITEs Bob receives in one scenario: accept, 486, CANCEL, the
/// held call and the 503.
const INITIAL_INVITES: u64 = 5;

struct Decider;

#[async_trait::async_trait]
impl CallHandler for Decider {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        if call.to.contains("reject") {
            CallHandlerDecision::Reject {
                status: 486,
                reason: "Busy Here".to_string(),
            }
        } else if call.to.contains("ring") {
            CallHandlerDecision::Defer(call.defer(Duration::from_secs(20)))
        } else {
            CallHandlerDecision::Accept
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Arm {
    Defaults,
    FastPaths,
}

impl Arm {
    fn apply(self, config: Config) -> Config {
        match self {
            Arm::Defaults => config,
            Arm::FastPaths => config
                .with_server_call_admission_lock_free(true)
                .with_inbound_session_history(false)
                .with_inline_incoming_call_dispatch(true),
        }
    }
}

fn peer_config(arm: Arm, name: &str, port: u16, media_start: u16) -> Config {
    let mut config = Config::local(name, port);
    config.media_port_start = media_start;
    config.media_port_end = media_start + 99;
    // Config-owned switch for the dialog-layer counters read below.
    config.sip_udp_diagnostics = true;
    config.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: false,
        ..SipTraceConfig::default()
    };
    arm.apply(config.trace_passthrough_for_development())
}

/// `Outbound INVITE 1 INVITE`, `Inbound 487 1 INVITE`, ...; `None` for 100.
fn normalize(direction: String, start_line: &str, raw: &str) -> Option<String> {
    let head = match start_line.strip_prefix("SIP/2.0 ") {
        Some(status) => status.split_whitespace().next()?.to_string(),
        None => start_line.split_whitespace().next()?.to_string(),
    };
    if head == "100" {
        return None;
    }
    let cseq = raw.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("CSeq")
            .then(|| value.split_whitespace().collect::<Vec<_>>().join(" "))
    })?;
    Some(format!("{direction} {head} {cseq}"))
}

#[derive(Default)]
struct Observed {
    lines: Vec<String>,
}

impl Observed {
    fn mark(store: &Mutex<Observed>) -> usize {
        store.lock().unwrap().lines.len()
    }

    /// Lines since `mark`, sorted and without duplicates (retransmissions).
    fn since(store: &Mutex<Observed>, mark: usize) -> Vec<String> {
        let mut lines = store.lock().unwrap().lines[mark..].to_vec();
        lines.sort();
        lines.dedup();
        lines
    }
}

async fn wait_for_state(alice: &Arc<UnifiedCoordinator>, id: &CallId, wanted: CallState) {
    let handle = alice.session(id);
    for _ in 0..160 {
        if matches!(handle.state().await, Ok(state) if state == wanted) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("call {id:?} never reached {wanted:?}");
}

/// The first line starting with `prefix` observed after `mark`.
async fn wait_for_line(store: &Mutex<Observed>, mark: usize, prefix: &str) -> String {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if let Some(line) = store.lock().unwrap().lines[mark..]
            .iter()
            .find(|line| line.starts_with(prefix))
            .cloned()
        {
            return line;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no {prefix} observed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn settle_to_zero(alice: &Arc<UnifiedCoordinator>, bob: &Arc<UnifiedCoordinator>) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if alice.list_sessions().await.is_empty() && bob.list_sessions().await.is_empty() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "sessions did not converge to zero"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

struct ArmOutcome {
    transcript: Vec<String>,
    /// Initial INVITEs Bob's session shards dequeued.
    queued_incoming_calls: u64,
}

async fn scenario(arm: Arm, media_base: u16) -> ArmOutcome {
    // Bob holds his port before Alice probes, so they never share one.
    let (bob, bob_port) = retry_address_in_use("bob CallbackPeer::new", || {
        let bob_port = probe_udp_port();
        let bob_config = peer_config(arm, "equivalence-bob", bob_port, media_base)
            .with_auto_180_ringing(true)
            .with_server_call_admission_limit(1);
        async move {
            CallbackPeer::new(Decider, bob_config)
                .await
                .map(|bob| (bob, bob_port))
        }
    })
    .await;
    let bob_coordinator = Arc::clone(bob.coordinator());
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });

    let (alice, alice_port) = retry_address_in_use("alice UnifiedCoordinator::new", || {
        let alice_port = probe_udp_port();
        let config = peer_config(arm, "equivalence-alice", alice_port, media_base + 100);
        async move {
            UnifiedCoordinator::new(config)
                .await
                .map(|alice| (alice, alice_port))
        }
    })
    .await;
    let observed = Arc::new(Mutex::new(Observed::default()));
    let mut alice_events = alice.events().await.expect("alice events");
    let sink = Arc::clone(&observed);
    let collector = tokio::spawn(async move {
        while let Some(event) = alice_events.next().await {
            let line = match &event {
                Event::SipTrace(trace) => normalize(
                    format!("{:?}", trace.direction),
                    &trace.start_line,
                    &trace.raw_message,
                ),
                Event::CallAnswered { .. } => Some("app answered".to_string()),
                Event::CallFailed { status_code, .. } => Some(format!("app failed {status_code}")),
                Event::CallCancelled { .. } => Some("app cancelled".to_string()),
                Event::CallEnded { .. } => Some("app ended".to_string()),
                _ => None,
            };
            if let Some(line) = line {
                sink.lock().unwrap().lines.push(line);
            }
        }
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    // Both peers set the process-wide dialog diagnostics when built; count
    // this arm only.
    rvoip_sip_dialog::diagnostics::reset();

    let target = |user: &str| format!("sip:{user}@127.0.0.1:{bob_port}");
    let from = Some(format!("sip:alice@127.0.0.1:{alice_port}"));
    let mut transcript = Vec::new();

    // 1. Full call, re-INVITEs to hold and resume it, BYE from the caller.
    let mark = Observed::mark(&observed);
    let call = alice
        .invite(from.clone(), target("accept"))
        .send()
        .await
        .expect("invite");
    wait_for_line(&observed, mark, "app answered").await;
    wait_for_state(&alice, &call, CallState::Active).await;
    alice.session(&call).hold().await.expect("hold");
    wait_for_state(&alice, &call, CallState::OnHold).await;
    alice.session(&call).resume().await.expect("resume");
    wait_for_state(&alice, &call, CallState::Active).await;
    let reason = alice
        .session(&call)
        .hangup_and_wait(Some(WAIT))
        .await
        .expect("hangup");
    settle_to_zero(&alice, &bob_coordinator).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    transcript.push(format!("== full call (end reason {reason})"));
    transcript.extend(Observed::since(&observed, mark));

    // 2. Handler rejection.
    let mark = Observed::mark(&observed);
    alice
        .invite(from.clone(), target("reject"))
        .send()
        .await
        .expect("invite");
    let failed = wait_for_line(&observed, mark, "app failed").await;
    settle_to_zero(&alice, &bob_coordinator).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    transcript.push(format!("== handler rejection ({failed})"));
    transcript.extend(Observed::since(&observed, mark));

    // 3. CANCEL while ringing.
    let mark = Observed::mark(&observed);
    let call = alice
        .invite(from.clone(), target("ring"))
        .send()
        .await
        .expect("invite");
    wait_for_state(&alice, &call, CallState::Ringing).await;
    let reason = alice
        .session(&call)
        .hangup_and_wait(Some(WAIT))
        .await
        .expect("cancel");
    settle_to_zero(&alice, &bob_coordinator).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    transcript.push(format!("== cancel (end reason {reason})"));
    transcript.extend(Observed::since(&observed, mark));

    // 4. Admission rejection behind one held call.
    let mark = Observed::mark(&observed);
    let held = alice
        .invite(from.clone(), target("accept"))
        .send()
        .await
        .expect("invite");
    wait_for_line(&observed, mark, "app answered").await;
    let second = Observed::mark(&observed);
    alice
        .invite(from.clone(), target("accept"))
        .send()
        .await
        .expect("invite");
    let rejected = wait_for_line(&observed, second, "app failed").await;
    let reason = alice
        .session(&held)
        .hangup_and_wait(Some(WAIT))
        .await
        .expect("hangup");
    settle_to_zero(&alice, &bob_coordinator).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    transcript.push(format!(
        "== admission ({rejected}; held end reason {reason})"
    ));
    transcript.extend(Observed::since(&observed, mark));

    // 5. Teardown converged (asserted by settle_to_zero above).
    transcript.push("== converged".to_string());
    let queued_incoming_calls =
        rvoip_sip_dialog::diagnostics::snapshot().dialog_to_session_queue_incoming_call;

    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob_task).await;
    let _ = alice
        .shutdown_gracefully(Some(Duration::from_secs(2)))
        .await;
    collector.abort();
    ArmOutcome {
        transcript,
        queued_incoming_calls,
    }
}

/// The transcript both arms must produce. It is also what the defaults
/// produced before any of the fast paths existed.
const EXPECTED: &[&str] = &[
    "== full call (end reason Local hangup)",
    "Inbound 180 1 INVITE",
    "Inbound 200 1 INVITE",
    "Inbound 200 2 INVITE",
    "Inbound 200 3 INVITE",
    "Inbound 200 4 BYE",
    "Outbound ACK 1 ACK",
    "Outbound ACK 2 ACK",
    "Outbound ACK 3 ACK",
    "Outbound BYE 4 BYE",
    "Outbound INVITE 1 INVITE",
    "Outbound INVITE 2 INVITE",
    "Outbound INVITE 3 INVITE",
    "app answered",
    "app ended",
    "== handler rejection (app failed 486)",
    "Inbound 180 1 INVITE",
    "Inbound 486 1 INVITE",
    "Outbound ACK 1 ACK",
    "Outbound INVITE 1 INVITE",
    "app failed 486",
    "== cancel (end reason Cancelled)",
    "Inbound 180 1 INVITE",
    "Inbound 200 1 CANCEL",
    "Inbound 487 1 INVITE",
    "Outbound ACK 1 ACK",
    "Outbound CANCEL 1 CANCEL",
    "Outbound INVITE 1 INVITE",
    "app cancelled",
    "== admission (app failed 503; held end reason Local hangup)",
    "Inbound 180 1 INVITE",
    "Inbound 200 1 INVITE",
    "Inbound 200 2 BYE",
    "Inbound 503 1 INVITE",
    "Outbound ACK 1 ACK",
    "Outbound BYE 2 BYE",
    "Outbound INVITE 1 INVITE",
    "app answered",
    "app ended",
    "app failed 503",
    "== converged",
];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inbound_fast_paths_preserve_wire_messages_and_outcomes() {
    let defaults = scenario(Arm::Defaults, 28_400).await;
    let fast_paths = scenario(Arm::FastPaths, 28_600).await;

    assert_eq!(
        defaults.transcript, fast_paths.transcript,
        "SIP wire messages or outcomes differ between the arms"
    );
    assert_eq!(defaults.transcript, EXPECTED, "unexpected transcript");

    // Every initial INVITE crosses a session shard by default; inline
    // dispatch handles each one on the dialog worker instead.
    assert_eq!(defaults.queued_incoming_calls, INITIAL_INVITES);
    assert_eq!(fast_paths.queued_incoming_calls, 0);
}
