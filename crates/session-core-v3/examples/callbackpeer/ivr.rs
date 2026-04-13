//! Interactive Voice Response (IVR) menu using DTMF.
//!
//!   cargo run --example callbackpeer_ivr
//!
//! Accepts calls and presents a DTMF menu:
//!   Press 1 for Sales
//!   Press 2 for Support
//!   Press 9 to hang up

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use rvoip_session_core_v3::{
    CallHandler, CallHandlerDecision, CallId, CallbackPeer, Config, EndReason, IncomingCall,
    SessionHandle, StreamPeer,
};

/// Per-call state tracking which menu option was selected.
struct IvrHandler {
    calls: Arc<Mutex<HashMap<String, String>>>,
}

impl IvrHandler {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl CallHandler for IvrHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!("[IVR] Incoming call from {}", call.from);
        self.calls
            .lock()
            .await
            .insert(call.call_id.to_string(), "main_menu".into());
        CallHandlerDecision::Accept
    }

    async fn on_call_established(&self, handle: SessionHandle) {
        println!("[IVR] Call {} connected — presenting menu", handle.id());
        println!("[IVR]   Press 1 for Sales");
        println!("[IVR]   Press 2 for Support");
        println!("[IVR]   Press 9 to hang up");
    }

    async fn on_dtmf(&self, handle: SessionHandle, digit: char) {
        let call_id = handle.id().to_string();
        println!("[IVR] Call {} pressed '{}'", call_id, digit);

        match digit {
            '1' => {
                println!("[IVR] -> Routing to Sales");
                self.calls.lock().await.insert(call_id, "sales".into());
            }
            '2' => {
                println!("[IVR] -> Routing to Support");
                self.calls.lock().await.insert(call_id, "support".into());
            }
            '9' => {
                println!("[IVR] -> Caller requested hangup");
                let _ = handle.hangup().await;
            }
            _ => {
                println!("[IVR] -> Invalid option, try again");
            }
        }
    }

    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {
        let state = self.calls.lock().await.remove(&call_id.to_string());
        println!(
            "[IVR] Call {} ended ({:?}), was in: {:?}",
            call_id, reason, state
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    // --- Background: test caller that dials in and presses DTMF digits ---
    tokio::spawn(async {
        sleep(Duration::from_secs(1)).await;
        let mut caller = StreamPeer::with_config(Config::local("caller", 5061)).await.unwrap();

        println!("[TEST] Calling IVR...");
        let handle = caller.call("sip:ivr@127.0.0.1:5060").await.unwrap();
        caller.wait_for_answered(handle.id()).await.ok();

        // Navigate the menu
        for digit in ['1', '2', '9'] {
            sleep(Duration::from_secs(1)).await;
            println!("[TEST] Pressing '{}'", digit);
            handle.send_dtmf(digit).await.ok();
        }

        sleep(Duration::from_secs(2)).await;
        handle.hangup().await.ok();
        println!("[TEST] Done.");
        sleep(Duration::from_secs(1)).await;
        std::process::exit(0);
    });

    // --- Demo: IVR server ---
    println!("IVR server on port 5060...");
    let peer = CallbackPeer::new(IvrHandler::new(), Config::local("ivr", 5060)).await?;
    peer.run().await?;
    Ok(())
}
