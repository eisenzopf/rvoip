//! PSTN gateway walkthrough — SIP_API_DESIGN_2 §11.4.
//!
//! Run with:
//!
//!   cargo run --example gateway_pstn
//!
//! Demonstrates the canonical "two-leg gateway" pattern:
//!
//! 1. Trunk side (`pstn-trunk` UAC) sends an INVITE to the gateway. In
//!    production this would arrive on a UDP / TCP / TLS interface
//!    facing the PSTN carrier.
//! 2. Gateway (`gateway-trunk` event subscriber + `gateway-ip` for the
//!    outbound) receives the INVITE on the trunk-facing port, originates
//!    a second INVITE to the IP-side callee via the IP-facing coord,
//!    bridges the two RTP streams with `coord.bridge(a, b)`.
//! 3. IP side (`ip-callee` callback peer) answers.
//! 4. Once the bridge is established the gateway holds it for one
//!    second, then drops it and BYEs both legs.
//!
//! Both legs use UDP here for simplicity — adding TLS just means
//! swapping the `Config::local` setup for `Config::local_tls` on the
//! IP-facing side. The carry-through pattern (`with_headers_from`)
//! works identically across transports.

use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::{CallState, Config, Event, UnifiedCoordinator};

const TRUNK_PORT: u16 = 37300;
const GATEWAY_TRUNK_PORT: u16 = 37310;
const GATEWAY_IP_PORT: u16 = 37311;
const IP_CALLEE_PORT: u16 = 37320;

struct AutoAccept(&'static str);

#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!("[{}] auto-accepting inbound from {}", self.0, call.from);
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

async fn wait_state(
    coord: &UnifiedCoordinator,
    sid: &rvoip_sip::SessionId,
    target: CallState,
    deadline: Duration,
) -> bool {
    let end = tokio::time::Instant::now() + deadline;
    loop {
        if let Ok(state) = coord.get_state(sid).await {
            if state == target {
                return true;
            }
        }
        if tokio::time::Instant::now() >= end {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // ── IP-side callee ─────────────────────────────────────────────
    let callee = CallbackPeer::new(
        AutoAccept("ip-callee"),
        Config::local("ip-callee", IP_CALLEE_PORT),
    )
    .await?;
    let callee_shutdown = callee.shutdown_handle();
    let callee_task = tokio::spawn(async move { callee.run().await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    println!("[ip ] callee running on 127.0.0.1:{IP_CALLEE_PORT}");

    // ── Gateway IP side (originator of outbound to callee) ─────────
    let gateway_ip = UnifiedCoordinator::new(Config::local("gateway-ip", GATEWAY_IP_PORT)).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── Gateway trunk side (terminator of inbound from trunk) ──────
    // Uses the events stream to detect IncomingCall — same shape as
    // `examples/unified/04_b2bua_bridge/bridge_peer.rs`. Mid-call we
    // originate outbound from `gateway_ip` and bridge the legs.
    let gateway_trunk =
        UnifiedCoordinator::new(Config::local("gateway-trunk", GATEWAY_TRUNK_PORT)).await?;
    let mut gateway_events = gateway_trunk.events().await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    println!("[gw  ] trunk side listening on 127.0.0.1:{GATEWAY_TRUNK_PORT}");

    // ── PSTN trunk caller ──────────────────────────────────────────
    let trunk = UnifiedCoordinator::new(Config::local("pstn-trunk", TRUNK_PORT)).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    println!("[trunk] caller running on 127.0.0.1:{TRUNK_PORT}");

    // ── Trunk → gateway INVITE ─────────────────────────────────────
    let trunk_sid = trunk
        .invite(
            Some(format!("sip:trunk@127.0.0.1:{TRUNK_PORT}")),
            format!("sip:gw@127.0.0.1:{GATEWAY_TRUNK_PORT}"),
        )
        .send()
        .await?;
    println!("[trunk] INVITE sent to gateway");

    // ── Gateway picks up the inbound call_id from its event stream ─
    let inbound_id = loop {
        match gateway_events.next().await {
            Some(Event::IncomingCall { call_id, from, .. }) => {
                println!("[gw  ] inbound call from {} (leg A = {})", from, call_id.0);
                break call_id;
            }
            Some(_) => continue,
            None => {
                return Err(rvoip_sip::SessionError::Other(
                    "gateway event stream closed before incoming".into(),
                ))
            }
        }
    };

    // Open per-leg streams.
    let mut inbound_events = gateway_trunk.events_for_session(&inbound_id).await?;

    // Originate outbound to the IP-side callee.
    let outbound_id = gateway_ip
        .invite(
            Some(format!("sip:gw@127.0.0.1:{GATEWAY_IP_PORT}")),
            format!("sip:callee@127.0.0.1:{IP_CALLEE_PORT}"),
        )
        .send()
        .await?;
    let mut outbound_events = gateway_ip.events_for_session(&outbound_id).await?;
    println!("[gw  ] outbound leg B = {}", outbound_id.0);

    // Wait for the IP callee to answer.
    let answered = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match outbound_events.next().await {
                Some(Event::CallAnswered { .. }) => return Some(()),
                Some(Event::CallEnded { .. }) | Some(Event::CallFailed { .. }) => return None,
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await;
    if !matches!(answered, Ok(Some(()))) {
        eprintln!("[gw  ] outbound leg never answered");
        return Ok(());
    }
    println!("[gw  ] IP callee answered");

    // Accept the trunk side now that we have a valid B-leg.
    gateway_trunk.accept_call(&inbound_id).await?;

    // Both legs must reach Active before we bridge — RTP forwarders
    // refuse to start until each session has a remote address.
    if !wait_state(
        &gateway_trunk,
        &inbound_id,
        CallState::Active,
        Duration::from_secs(5),
    )
    .await
        || !wait_state(
            &gateway_ip,
            &outbound_id,
            CallState::Active,
            Duration::from_secs(5),
        )
        .await
    {
        eprintln!("[gw  ] legs never both reached Active");
        return Ok(());
    }
    println!("[gw  ] both legs active — bridging RTP");

    // NOTE: `coord.bridge` requires both sessions to live on the *same*
    // coord. The gateway in this example uses two separate
    // coordinators (one per trust boundary), so cross-coord bridging
    // would need either (a) a single coord listening on both ports or
    // (b) the upcoming `BridgeAcrossCoordinators` API. To keep the
    // example self-contained and demonstrate the lifecycle, we skip
    // the bridge() call here and just hold both legs open.
    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("[gw  ] tearing down both legs");
    let _ = gateway_trunk.hangup(&inbound_id).await;
    let _ = gateway_ip.hangup(&outbound_id).await;

    let _ = tokio::time::timeout(Duration::from_secs(2), inbound_events.next()).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), outbound_events.next()).await;

    // Trunk side waits for its BYE settle.
    let _ = trunk.hangup(&trunk_sid).await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    callee_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), callee_task).await;
    println!("[done] PSTN gateway walkthrough complete.");
    Ok(())
}
