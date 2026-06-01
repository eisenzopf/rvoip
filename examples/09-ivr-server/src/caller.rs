//! Test caller for the reactive IVR server.
//!
//! Connects to the IVR, sends a short DTMF sequence (1, 2, #) to exercise the
//! server's `on_dtmf` hook, then hangs up.
//!
//! Run with `./run_demo.sh`, or pair manually with the `server` binary.

use std::time::Duration;

use rvoip_sip::{Config, StreamPeer};

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();

    let mut caller = StreamPeer::with_config(Config::local("caller", 5121)).await?;

    let call_id = caller.invite("sip:ivr@127.0.0.1:5120").send().await?;
    let call = caller.coordinator().session(&call_id);
    caller.wait_for_answered(call.id()).await?;
    println!("[caller] ✅ connected to IVR");

    for digit in ['1', '2', '#'] {
        call.send_dtmf(digit).await?;
        println!("[caller] sent DTMF {digit}");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    println!("[caller] ✅ done");
    caller.shutdown().await
}
