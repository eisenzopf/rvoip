//! CallbackPeer builder IVR example.
//!
//! Run with:
//!
//!   cargo run --example callbackpeer_builder_ivr

use rvoip_session_core::{CallHandlerDecision, CallbackPeer, Config, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let peer = CallbackPeer::builder(Config::local("ivr", 5090))
        .on_incoming(|call| async move {
            println!("incoming call from {}", call.from);
            CallHandlerDecision::Accept
        })
        .on_established(|call| async move {
            println!("call {} established", call.id());
            Ok(())
        })
        .on_dtmf(|call, digit| async move {
            println!("call {} pressed {}", call.id(), digit);
            if digit == '0' {
                call.transfer_blind("sip:operator@127.0.0.1:5091").await?;
            }
            Ok(())
        })
        .on_ended(|call_id, reason| async move {
            println!("call {} ended: {:?}", call_id, reason);
            Ok(())
        })
        .build()
        .await?;

    println!("IVR listening on sip:ivr@127.0.0.1:5090");
    peer.run().await
}
