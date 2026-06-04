//! SBC topology-hiding walkthrough — SIP_API_DESIGN_2 §11.3.
//!
//! Run with:
//!
//!   cargo run --example sbc_topology_hiding
//!
//! Boots three coordinators in-process:
//!
//! - **alice** — upstream UAC. Sends an INVITE with application headers
//!   (`History-Info`, `Diversion`, `X-Customer-ID`), `Privacy: id;header`,
//!   and a sensitive `P-Asserted-Identity` from the untrusted side.
//! - **sbc** — middle. Receives `IncomingCall`, drives an outbound INVITE
//!   to bob via `with_headers_from(&call, ...)` carry-through, strips
//!   `Privacy`, and rewrites `P-Asserted-Identity` per the §11.3
//!   trust-boundary pattern.
//! - **bob** — downstream UAS. Receives the SBC's outbound INVITE.
//!
//! Additionally, alice configures a `TraceRedactor` that drops
//! `Authorization:` from `SipTrace` output even though it stays on the
//! wire — the §11.3 "redact in observability, never on the wire"
//! guarantee.
//!
//! The example completes when bob's wire trace is captured and printed
//! with the carry-through / strip / rewrite outputs visible.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::trace_redactor::{RedactionDecision, TraceRedactor};
use rvoip_sip::{
    Config, Event, HeaderName, SipRequestOptions, SipTraceConfig, SipTraceDirection,
    UnifiedCoordinator,
};

const ALICE_PORT: u16 = 36100;
const SBC_PORT: u16 = 36110;
const BOB_PORT: u16 = 36120;

// Application headers carried through trust boundary.
const HISTORY_INFO: &str = "<sip:reception@upstream.example>;index=1";
const DIVERSION: &str = "<sip:menu@upstream.example>;reason=no-answer";
const CUSTOMER_ID: &str = "cust-7142";
const PRIVACY_VALUE: &str = "id;header";

// PAI rewrite: untrusted upstream value gets replaced before egress.
const UNTRUSTED_PAI: &str = "<sip:upstream-trunk@untrusted.example>";
const REWRITTEN_PAI: &str = "<sip:sbc-rewritten@sbc.example>";

// Sensitive header that traces must redact but the wire must keep.
// We use a custom `X-Internal-Token` (application-controlled, no policy
// restriction) rather than `Authorization` — the canonical INVITE
// builder routes Authorization through `with_credentials` / `with_auth`
// instead of `with_raw_header`, so this example uses the custom header to
// demonstrate the redactor without tripping the §5.1 policy guard.
const SECRET_TOKEN: &str = "tok-deadbeef-secret";
const TOKEN_HEADER: &str = "X-Internal-Token";

#[derive(Debug)]
struct DropSensitiveTokens;

impl TraceRedactor for DropSensitiveTokens {
    fn redact(&self, header: &HeaderName, _value: &str) -> RedactionDecision {
        match header {
            HeaderName::Authorization => RedactionDecision::Drop,
            HeaderName::Other(name) if name.eq_ignore_ascii_case(TOKEN_HEADER) => {
                RedactionDecision::Drop
            }
            _ => RedactionDecision::Keep,
        }
    }
}

/// SBC carry-through handler. Drives the outbound leg using the inbound
/// `IncomingCall` as the `SipHeaderView` source. `fired` guards against
/// duplicate `on_incoming_call` invocations from inbound retransmits —
/// the trust-boundary rewrite must run exactly once per upstream INVITE.
struct SbcHandler {
    outbound_coord: Arc<UnifiedCoordinator>,
    fired: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl CallHandler for SbcHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        if self.fired.swap(true, Ordering::SeqCst) {
            return CallHandlerDecision::Reject {
                status: 503,
                reason: "SBC example duplicate".to_string(),
            };
        }
        let names = [
            HeaderName::Other("History-Info".to_string()),
            HeaderName::Other("Diversion".to_string()),
            HeaderName::Other("X-Customer-ID".to_string()),
            HeaderName::Other("Privacy".to_string()),
        ];

        let target = format!("sip:bob@127.0.0.1:{BOB_PORT}");
        let from = format!("sip:sbc@127.0.0.1:{SBC_PORT}");

        let builder = self.outbound_coord.invite(Some(from), target);

        let (chain, report) = match builder.with_headers_from(&call, &names) {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!("[SBC] with_headers_from failed: {err:?}");
                return CallHandlerDecision::Reject {
                    status: 500,
                    reason: "Carry-through failed".to_string(),
                };
            }
        };

        println!("[SBC] carry-through report:");
        println!("        copied : {:?}", report.copied);
        println!("        skipped: {:?}", report.skipped);

        let chain = chain
            .strip_header(&HeaderName::Other("Privacy".to_string()))
            .with_raw_header(
                HeaderName::Other("P-Asserted-Identity".to_string()),
                REWRITTEN_PAI,
            )
            .expect("rewrite PAI");

        if let Err(err) = chain.send().await {
            eprintln!("[SBC] outbound send failed: {err:?}");
        }

        // Give the outbound a moment to reach the wire before we
        // reject the inbound leg. The example only asserts on bob's
        // wire trace, so a clean 503 is fine.
        tokio::time::sleep(Duration::from_millis(100)).await;
        CallHandlerDecision::Reject {
            status: 503,
            reason: "SBC example complete".to_string(),
        }
    }
}

fn config(name: &str, port: u16) -> Config {
    let mut c = Config::local(name, port);
    c.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: true,
        ..SipTraceConfig::default()
    };
    c
}

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // ── Bob (downstream UAS) ───────────────────────────────────────
    let bob = UnifiedCoordinator::new(config("bob", BOB_PORT)).await?;
    let mut bob_events = bob.events().await?;
    tokio::time::sleep(Duration::from_millis(150)).await;

    // ── SBC middle ─────────────────────────────────────────────────
    // The same coord is used for both inbound and outbound — minimal
    // viable B2BUA shape. A production SBC would typically use two
    // coordinators (one per trust boundary).
    let sbc = UnifiedCoordinator::new(config("sbc-outbound", SBC_PORT)).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let handler = SbcHandler {
        outbound_coord: sbc.clone(),
        fired: Arc::new(AtomicBool::new(false)),
    };
    let mut sbc_peer_cfg = config("sbc-inbound", SBC_PORT + 1);
    // Same trust-boundary redactor on the SBC's inbound leg.
    sbc_peer_cfg.trace_redaction = Some(Arc::new(DropSensitiveTokens));
    let sbc_peer = CallbackPeer::new(handler, sbc_peer_cfg).await?;
    let sbc_shutdown = sbc_peer.shutdown_handle();
    let sbc_task = tokio::spawn(async move {
        let _ = sbc_peer.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    // ── Alice (upstream UAC) ───────────────────────────────────────
    let mut alice_cfg = config("alice", ALICE_PORT);
    alice_cfg.trace_redaction = Some(Arc::new(DropSensitiveTokens));
    let alice = UnifiedCoordinator::new(alice_cfg).await?;
    let mut alice_events = alice.events().await?;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", SBC_PORT + 1);
    let _call_id = alice
        .invite(Some(format!("sip:alice@127.0.0.1:{ALICE_PORT}")), target)
        .with_raw_header(HeaderName::Other("History-Info".to_string()), HISTORY_INFO)
        .expect("History-Info")
        .with_raw_header(HeaderName::Other("Diversion".to_string()), DIVERSION)
        .expect("Diversion")
        .with_raw_header(HeaderName::Other("X-Customer-ID".to_string()), CUSTOMER_ID)
        .expect("X-Customer-ID")
        .with_raw_header(HeaderName::Other("Privacy".to_string()), PRIVACY_VALUE)
        .expect("Privacy")
        .with_raw_header(
            HeaderName::Other("P-Asserted-Identity".to_string()),
            UNTRUSTED_PAI,
        )
        .expect("upstream PAI")
        .with_raw_header(HeaderName::Other(TOKEN_HEADER.to_string()), SECRET_TOKEN)
        .expect("internal token")
        .send()
        .await?;

    println!("[alice] INVITE sent with {} app headers", 5);

    // Drain bob's inbound INVITE trace. The global event bus surfaces
    // SipTrace events from every coord in the process, so we filter on
    // `local_addr` to pick out the trace that actually arrived on bob's
    // transport (port BOB_PORT) — not alice's trace of the same INVITE.
    let trace = wait_inbound_on(
        &mut bob_events,
        "INVITE",
        &format!("127.0.0.1:{BOB_PORT}"),
        Duration::from_secs(10),
    )
    .await;
    match trace {
        Some(line) => {
            println!("[bob ] inbound INVITE wire:");
            for line in line.lines().take(20) {
                println!("        {line}");
            }
            check(
                line.contains(HISTORY_INFO),
                "carry-through: History-Info on wire",
            );
            check(line.contains(DIVERSION), "carry-through: Diversion on wire");
            check(
                line.contains(CUSTOMER_ID),
                "carry-through: X-Customer-ID on wire",
            );
            check(!line.contains(PRIVACY_VALUE), "strip: Privacy not on wire");
            check(line.contains(REWRITTEN_PAI), "rewrite: PAI rewritten");
            check(
                !line.contains(UNTRUSTED_PAI),
                "rewrite: upstream PAI absent",
            );
        }
        None => eprintln!("[bob ] never saw inbound INVITE within 10s"),
    }

    // Demonstrate redactor effect: drain alice's outbound trace and
    // confirm Authorization is gone from the trace (but the SECRET_AUTH
    // value above did land on the wire, since the redactor only
    // affects observability, not transport).
    let alice_trace = wait_outbound_on(
        &mut alice_events,
        "INVITE",
        &format!("127.0.0.1:{ALICE_PORT}"),
        Duration::from_secs(2),
    )
    .await;
    if let Some(line) = alice_trace {
        check(
            !line.contains(SECRET_TOKEN),
            "redactor: X-Internal-Token payload dropped from trace",
        );
    }

    sbc_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), sbc_task).await;
    println!("[done] SBC topology-hiding walkthrough complete.");
    Ok(())
}

fn check(cond: bool, label: &str) {
    if cond {
        println!("[check] ✓ {label}");
    } else {
        println!("[check] ✗ {label}");
    }
}

async fn wait_inbound_on(
    events: &mut rvoip_sip::EventReceiver,
    method: &str,
    local_addr: &str,
    timeout: Duration,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return None,
            Ok(Some(Event::SipTrace(trace))) => {
                if trace.direction == SipTraceDirection::Inbound
                    && trace.start_line.starts_with(method)
                    && trace.local_addr == local_addr
                {
                    return Some(trace.raw_message);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}

async fn wait_outbound_on(
    events: &mut rvoip_sip::EventReceiver,
    method: &str,
    local_addr: &str,
    timeout: Duration,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return None,
            Ok(Some(Event::SipTrace(trace))) => {
                if trace.direction == SipTraceDirection::Outbound
                    && trace.start_line.starts_with(method)
                    && trace.local_addr == local_addr
                {
                    return Some(trace.raw_message);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}
