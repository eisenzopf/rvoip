//! Full CallHandler implementation showing every callback method.
//!
//!   cargo run --example callbackpeer_custom
//!
//! Reference implementation demonstrating all 5 CallHandler methods:
//!   on_incoming_call, on_call_established, on_call_ended, on_dtmf, on_transfer_request

use async_trait::async_trait;
use tokio::time::{sleep, Duration};

use rvoip_session_core_v3::{
    CallHandler, CallHandlerDecision, CallId, CallbackPeer, Config, EndReason, IncomingCall,
    SessionHandle, StreamPeer,
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
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    // --- Background: test caller that exercises on_established, on_dtmf, on_call_ended ---
    tokio::spawn(async {
        sleep(Duration::from_secs(1)).await;
        let mut caller = StreamPeer::with_config(Config::local("caller", 5061)).await.unwrap();

        println!("[TEST] Calling custom handler...");
        let handle = caller.call("sip:custom@127.0.0.1:5060").await.unwrap();
        caller.wait_for_answered(handle.id()).await.ok();

        // Send some DTMF to trigger on_dtmf
        for digit in ['5', '#'] {
            sleep(Duration::from_secs(1)).await;
            println!("[TEST] Sending DTMF '{}'", digit);
            handle.send_dtmf(digit).await.ok();
        }

        sleep(Duration::from_secs(2)).await;
        println!("[TEST] Hanging up...");
        handle.hangup().await.ok();
        caller.wait_for_ended(handle.id()).await.ok();

        println!("[TEST] Done.");
        sleep(Duration::from_secs(1)).await;
        std::process::exit(0);
    });

    // --- Demo: custom handler server ---
    println!("Custom handler server on port 5060...");
    let peer = CallbackPeer::new(MyHandler, Config::local("custom", 5060)).await?;
    peer.run().await?;
    Ok(())
}
