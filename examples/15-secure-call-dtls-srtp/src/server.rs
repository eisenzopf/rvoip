//! Secure call (DTLS-SRTP) — the **server**.
//!
//! With `Config::offer_srtp = true` and `Config::srtp_keying =
//! SrtpKeyingMode::DtlsSrtp` the media adapter answers with
//! `m=audio … UDP/TLS/RTP/SAVP …` plus `a=fingerprint`/`a=setup` (RFC
//! 5763/5764) instead of SDES's `a=crypto:`. The actual SRTP keys come
//! from a real DTLS 1.2 handshake run over the RTP port *after* the 200
//! OK, so `on_call_established` firing doesn't yet mean media is
//! encrypted — this demo also subscribes to the raw event stream to
//! observe `Event::MediaSecurityNegotiated` once the handshake actually
//! completes, independent of call setup.
//!
//! Run with `./run_demo.sh`, or pair manually with the `client` binary.

use async_trait::async_trait;
use rvoip_sip::{
    CallHandler, CallHandlerDecision, CallId, CallbackPeer, Config, EndReason, Event, IncomingCall,
    SessionHandle,
};

struct DtlsSrtpLogger;

#[async_trait]
impl CallHandler for DtlsSrtpLogger {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!(
            "[SERVER] Incoming DTLS-SRTP call: {} -> {}",
            call.from, call.to
        );
        CallHandlerDecision::Accept
    }

    async fn on_call_established(&self, handle: SessionHandle) {
        println!(
            "[SERVER] Call {} established (signaling only — DTLS handshake runs in the background)",
            handle.id()
        );
    }

    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {
        println!("[SERVER] Call {call_id} ended: {reason:?}");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

    let mut config = Config::local("dtls_server", 5060);
    config.offer_srtp = true;
    config.srtp_keying = rvoip_sip::api::unified::SrtpKeyingMode::DtlsSrtp;

    let peer = CallbackPeer::new(DtlsSrtpLogger, config).await?;

    // CallHandler has no dedicated hook for MediaSecurityNegotiated, so
    // watch the raw event stream directly to report when the DTLS
    // handshake actually completes.
    let coordinator = peer.coordinator().clone();
    tokio::spawn(async move {
        if let Ok(mut events) = coordinator.events().await {
            while let Some(event) = events.next().await {
                if let Event::MediaSecurityNegotiated {
                    keying,
                    suite,
                    profile,
                    contexts_installed,
                    ..
                } = event
                {
                    println!(
                        "[SERVER] media secured — keying={keying:?} suite={suite:?} \
                         profile={profile:?} contexts_installed={contexts_installed}"
                    );
                }
            }
        }
    });

    println!("Listening on 5060 with DTLS-SRTP (UDP/TLS/RTP/SAVP + a=fingerprint)…");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
