//! Auto-answer SIP server that logs each RFC 4733 DTMF digit it
//! receives. Proves the Sprint 2 B4 receive path is wired end-to-end:
//! rtp-core PT 101 decode → media-core callback → session-core
//! `on_dtmf(handle, digit)`.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_dtmf_server
//! Or with client:  ./examples/streampeer/dtmf/run.sh

use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rvoip_session_core::{
    CallHandler, CallHandlerDecision, CallId, CallbackPeer, Config, EndReason, IncomingCall,
    SessionHandle,
};

/// Accepts every call and prints each inbound DTMF event. Digits are
/// counted so the example harness can flag a regression if the client
/// sends N digits but none are observed here.
struct DtmfLogger {
    received: Arc<AtomicUsize>,
}

#[async_trait]
impl CallHandler for DtmfLogger {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!("[SERVER] Incoming call: {} -> {}", call.from, call.to);
        CallHandlerDecision::Accept
    }

    async fn on_call_established(&self, handle: SessionHandle) {
        println!("[SERVER] Call {} established — waiting for DTMF…", handle.id());
    }

    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {
        println!(
            "[SERVER] Call {} ended ({:?}); DTMF digits seen: {}",
            call_id,
            reason,
            self.received.load(Ordering::SeqCst)
        );
    }

    async fn on_dtmf(&self, handle: SessionHandle, digit: char) {
        let n = self.received.fetch_add(1, Ordering::SeqCst) + 1;
        println!("[SERVER] #{} DTMF '{}' on call {}", n, digit, handle.id());
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let handler = DtmfLogger {
        received: Arc::new(AtomicUsize::new(0)),
    };
    let peer = CallbackPeer::new(handler, Config::local("server", 5060)).await?;

    println!("Listening on port 5060 (DTMF logger)...");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
