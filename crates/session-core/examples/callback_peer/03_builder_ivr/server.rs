//! CallbackPeer builder IVR server.
//!
//! Run with the client:
//!
//!   ./examples/callback_peer/03_builder_ivr/run.sh

use rvoip_session_core::{CallHandlerDecision, CallbackPeer, Config, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let peer = CallbackPeer::builder(Config::local("ivr", 5120))
        .on_incoming(|call| async move {
            println!("[ivr] incoming call from {}", call.from);
            CallHandlerDecision::Accept
        })
        .on_established(|call| async move {
            println!("[ivr] call {} established", call.id());
            Ok(())
        })
        .on_dtmf(|call, digit| async move {
            println!("[ivr] call {} pressed {}", call.id(), digit);
            if digit == '0' {
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
