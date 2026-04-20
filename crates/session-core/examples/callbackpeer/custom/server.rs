//! Full CallHandler implementation showing every callback method.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example callbackpeer_custom_server
//! Or with client:  ./examples/callbackpeer/custom/run.sh

use async_trait::async_trait;

use rvoip_session_core::{
    CallHandler, CallHandlerDecision, CallId, CallbackPeer, Config, EndReason, IncomingCall,
    SessionHandle,
};

struct MyHandler;

#[async_trait]
impl CallHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!("[HANDLER] Incoming: {} -> {}", call.from, call.to);
        CallHandlerDecision::Accept
    }

    async fn on_call_established(&self, handle: SessionHandle) {
        println!("[HANDLER] Call {} established", handle.id());
    }

    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {
        println!("[HANDLER] Call {} ended: {:?}", call_id, reason);
    }

    async fn on_dtmf(&self, handle: SessionHandle, digit: char) {
        println!("[HANDLER] Call {} received DTMF: {}", handle.id(), digit);
    }

    async fn on_transfer_request(&self, handle: SessionHandle, target: String) -> bool {
        println!(
            "[HANDLER] Call {} transfer request to {}",
            handle.id(),
            target
        );
        true
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let peer = CallbackPeer::new(MyHandler, Config::local("custom", 5060)).await?;

    println!("Listening on port 5060 (custom handler)...");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
