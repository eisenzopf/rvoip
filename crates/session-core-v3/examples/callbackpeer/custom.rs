//! Full CallHandler implementation showing every callback method.
//!
//!   cargo run --example callbackpeer_custom
//!
//! This is a reference implementation demonstrating all 5 CallHandler methods:
//!   on_incoming_call, on_call_established, on_call_ended, on_dtmf, on_transfer_request

use async_trait::async_trait;

use rvoip_session_core_v3::{
    CallHandler, CallHandlerDecision, CallId, CallbackPeer, Config, EndReason, IncomingCall,
    SessionHandle,
};

struct MyHandler;

#[async_trait]
impl CallHandler for MyHandler {
    /// Decide whether to accept, reject, redirect, or defer an incoming call.
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!("[HANDLER] Incoming: {} -> {}", call.from, call.to);
        // Accept all calls in this example
        CallHandlerDecision::Accept
    }

    /// Called when a call is fully connected (SDP negotiated, media ready).
    async fn on_call_established(&self, handle: SessionHandle) {
        println!("[HANDLER] Call {} established", handle.id());

        // You can interact with the call here:
        //   handle.audio().await    — get audio stream
        //   handle.hold().await     — put on hold
        //   handle.send_dtmf('1')   — send DTMF
    }

    /// Called when a call ends for any reason.
    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {
        println!("[HANDLER] Call {} ended: {:?}", call_id, reason);
    }

    /// Called when a DTMF digit is received.
    async fn on_dtmf(&self, handle: SessionHandle, digit: char) {
        println!("[HANDLER] Call {} received DTMF: {}", handle.id(), digit);
    }

    /// Called when a REFER (transfer request) is received.
    /// Return true to allow the transfer, false to reject.
    async fn on_transfer_request(&self, handle: SessionHandle, target: String) -> bool {
        println!(
            "[HANDLER] Call {} transfer request to {}",
            handle.id(),
            target
        );
        // Allow all transfers in this example
        true
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    println!("Custom handler server on port 5060...");
    let peer = CallbackPeer::new(MyHandler, Config::local("custom", 5060)).await?;
    peer.run().await?;
    Ok(())
}
