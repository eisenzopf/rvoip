//! Asterisk UDP DTMF caller: 2001 calls 2002 and sends configured digits.

#[path = "../common.rs"]
mod common;

use common::{
    call_with_answer_retry, endpoint_config, init_tracing, load_env, post_register_settle_duration,
    register_endpoint, remote_test_digits, remote_test_timeout, ExampleResult,
};
use rvoip_session_core::StreamPeer;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("2001", 5080, 17000, 17100)?;
    let mut peer = StreamPeer::with_config(cfg.stream_config()).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;

    let settle = post_register_settle_duration()?;
    if !settle.is_zero() {
        println!(
            "[2001] Waiting {}s for Asterisk OPTIONS qualify before calling...",
            settle.as_secs()
        );
        sleep(settle).await;
    }

    let target = cfg.outbound_call_uri("2002");
    let digits = remote_test_digits();
    println!("[2001] Calling {} for DTMF test.", target);
    let handle = call_with_answer_retry(&mut peer, &target, remote_test_timeout()?).await?;
    println!(
        "[2001] Connected; sending DTMF digits {}.",
        digits.iter().collect::<String>()
    );

    for digit in digits {
        sleep(Duration::from_millis(500)).await;
        println!("[2001] Sending DTMF '{}'.", digit);
        handle.send_dtmf(digit).await?;
    }

    sleep(Duration::from_secs(1)).await;
    handle.hangup().await?;
    handle.wait_for_end(Some(Duration::from_secs(8))).await.ok();

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[2001] Done.");
    Ok(())
}
