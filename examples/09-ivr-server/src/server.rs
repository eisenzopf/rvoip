//! Reactive IVR server (CallbackPeer builder).
//!
//! Where `StreamPeer` is sequential, [`CallbackPeer`] is reactive: you register
//! hooks and the library dispatches typed events into them. The builder form
//! shown here wires closures for the lifecycle a simple IVR cares about —
//! incoming-call gating, established, DTMF, and ended. Press `0` and the IVR
//! blind-transfers the caller to an operator.
//!
//! Run with `./run_demo.sh`, or pair manually with the `caller` binary.

use rvoip_sip::{CallHandlerDecision, CallbackPeer, Config, Result};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();

    let peer = CallbackPeer::builder(Config::local("ivr", 5120))
        .on_incoming(|call| async move {
            println!("[ivr] incoming call from {}", call.from);
            CallHandlerDecision::Accept
        })
        .on_established(|call| async move {
            println!("[ivr] ✅ call {} established", call.id());
            Ok(())
        })
        .on_dtmf(|call, digit| async move {
            println!("[ivr] call {} pressed {}", call.id(), digit);
            if digit == '0' {
                // Transfer to a human operator (not started in this 2-process
                // demo, so the caller only presses 1/2/#).
                call.transfer_blind("sip:operator@127.0.0.1:5122").await?;
            }
            Ok(())
        })
        .on_ended(|call_id, reason| async move {
            println!("[ivr] call {call_id} ended: {reason:?}");
            Ok(())
        })
        .build()
        .await?;

    println!("[ivr] listening on sip:ivr@127.0.0.1:5120");
    tokio::select! {
        res = peer.run() => res,
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}
