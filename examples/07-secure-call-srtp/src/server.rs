//! Secure call (SDES-SRTP) — the **server**.
//!
//! With `Config::offer_srtp = true` the media adapter adds `a=crypto:` to
//! outgoing SDP answers and installs paired `SrtpContext`s on both directions
//! after negotiation, so all RTP audio is AES-CM-128/HMAC-SHA1-80 protected on
//! the wire. `srtp_required = true` refuses plaintext fallback — an offer
//! without `a=crypto:` is rejected with a terminal failure, matching
//! Asterisk's `srtp=mandatory` / FreeSWITCH's `rtp_secure_media=mandatory`.
//!
//! Run with `./run_demo.sh`, or pair manually with the `client` binary.

use async_trait::async_trait;
use rvoip_sip::{
    CallHandler, CallHandlerDecision, CallId, CallbackPeer, Config, EndReason, IncomingCall,
    SessionHandle,
};

struct SrtpLogger;

#[async_trait]
impl CallHandler for SrtpLogger {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!(
            "[SERVER] Incoming SRTP-required call: {} -> {}",
            call.from, call.to
        );
        CallHandlerDecision::Accept
    }

    async fn on_call_established(&self, handle: SessionHandle) {
        println!("[SERVER] ✅ Call {} established with SRTP", handle.id());
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

    let mut config = Config::local("srtp_server", 5060);
    config.offer_srtp = true;
    config.srtp_required = true;

    let peer = CallbackPeer::new(SrtpLogger, config).await?;

    println!("Listening on 5060 with SRTP mandatory (RTP/SAVP + a=crypto:)…");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
