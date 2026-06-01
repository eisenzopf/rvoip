//! Full CallHandler implementation showing every callback method.
//!
//! Run standalone:  cargo run -p rvoip-sip --example callback_peer_trait_handler_server
//! Or with client:  ./examples/callback_peer/06_trait_handler/run.sh

use async_trait::async_trait;

use rvoip_sip::{
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

    async fn on_refer_received(&self, request: rvoip_sip::IncomingRequest) {
        println!(
            "[HANDLER] Call {} REFER received (method={:?})",
            request.call_id, request.method
        );
        // Auto-accept the transfer.
        let handle = match request.session_handle() {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[HANDLER] session_handle unavailable: {e}");
                return;
            }
        };
        if let Err(e) = handle.accept_refer().await {
            eprintln!("[HANDLER] accept_refer failed: {e}");
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
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
