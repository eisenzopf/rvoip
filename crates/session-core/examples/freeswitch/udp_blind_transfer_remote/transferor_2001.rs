//! FreeSWITCH UDP blind transfer test, transferor side.
//!
//! 2001 calls rvoip endpoint 2002 through FreeSWITCH, then sends REFER to
//! transfer 2002 to rvoip endpoint 2003. 2001 waits for REFER NOTIFY completion.

#[path = "../common.rs"]
mod common;

use common::{
    call_with_answer_retry, endpoint_config, env_duration_secs, init_tracing, load_env,
    post_register_settle_duration, register_endpoint, remote_test_timeout, ExampleResult,
};
use rvoip_session_core::{StreamPeer, TransferOutcome, TransferWaitMode};
use tokio::time::sleep;

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
            "[2001] Waiting {}s for FreeSWITCH OPTIONS qualify before calling...",
            settle.as_secs()
        );
        sleep(settle).await;
    }

    let call_target = cfg.outbound_call_uri("2002");
    let transfer_target = cfg.remote_call_uri();
    println!("[2001] Calling {} before transfer.", call_target);
    let handle = call_with_answer_retry(&mut peer, &call_target, remote_test_timeout()?).await?;
    println!(
        "[2001] Call established; transferring peer to rvoip target {}.",
        transfer_target
    );
    let transfer_settle = env_duration_secs("FREESWITCH_TRANSFER_SETTLE_SECS", 3);
    if !transfer_settle.is_zero() {
        println!(
            "[2001] Waiting {}s for FreeSWITCH bridge settle before REFER.",
            transfer_settle.as_secs()
        );
        sleep(transfer_settle).await;
    }

    let transfer_outcome = handle
        .transfer_blind_and_wait_for_outcome(
            &transfer_target,
            TransferWaitMode::NotifyFinal,
            Some(remote_test_timeout()?),
        )
        .await?;
    match transfer_outcome {
        TransferOutcome::ReferCompleted {
            status_code,
            reason,
            ..
        } => println!(
            "[2001] REFER completed with final NOTIFY: {} {}",
            status_code, reason
        ),
        TransferOutcome::Failed {
            status_code,
            reason,
            ..
        } => return Err(format!("REFER failed: {} {}", status_code, reason).into()),
        other => return Err(format!("unexpected transfer outcome: {:?}", other).into()),
    }
    handle
        .hangup_and_wait(Some(std::time::Duration::from_secs(8)))
        .await
        .ok();

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[2001] Transfer completed. Done.");
    Ok(())
}
