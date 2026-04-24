//! SRTP SIP server — negotiates RFC 4568 SDES-SRTP on inbound calls.
//!
//! Demonstrates the Sprint 1 B2 path: set `Config::offer_srtp = true`
//! and the media adapter adds `a=crypto:` to outgoing SDP answers, the
//! UdpRtpTransport installs paired `SrtpContext`s on both directions
//! after negotiation, and all RTP audio is AES-CM-128/HMAC-SHA1-80
//! protected on the wire.
//!
//! Setting `srtp_required = true` (as here) refuses to fall back to
//! plaintext — an offer without `a=crypto:` is rejected with a
//! terminal failure, matching Asterisk's `srtp=mandatory` /
//! FreeSWITCH's `rtp_secure_media=mandatory` behavior.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_srtp_server
//! Or with client:  ./examples/streampeer/srtp/run.sh

use async_trait::async_trait;
use rvoip_session_core::{
    CallHandler, CallHandlerDecision, CallId, CallbackPeer, Config, EndReason, IncomingCall,
    SessionHandle,
};

struct SrtpLogger;

#[async_trait]
impl CallHandler for SrtpLogger {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!("[SERVER] Incoming SRTP-required call: {} -> {}", call.from, call.to);
        CallHandlerDecision::Accept
    }

    async fn on_call_established(&self, handle: SessionHandle) {
        println!("[SERVER] Call {} established with SRTP", handle.id());
    }

    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {
        println!("[SERVER] Call {} ended: {:?}", call_id, reason);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
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
