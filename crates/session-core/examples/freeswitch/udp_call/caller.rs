#[path = "../common.rs"]
mod common;

use common::{
    call_with_answer_retry, endpoint_config, init_tracing, load_env, post_register_settle_duration,
    register_endpoint, ExampleResult,
};
use rvoip_session_core::StreamPeer;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("2001", 15080, 17000, 17100)?;
    let mut peer = StreamPeer::with_config(cfg.stream_config()).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;

    let settle = post_register_settle_duration()?;
    if !settle.is_zero() {
        println!(
            "[2001] Waiting {}s for FreeSWITCH registration propagation before calling...",
            settle.as_secs()
        );
        sleep(settle).await;
    }

    let target = cfg.outbound_call_uri("2002");
    println!("[2001] Calling {} for basic UDP call test.", target);
    let answered =
        call_with_answer_retry(&mut peer, &target, common::remote_test_timeout()?).await?;

    sleep(Duration::from_secs(2)).await;
    answered
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await?;

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    Ok(())
}
