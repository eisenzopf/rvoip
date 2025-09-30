//! UAC Caller - makes an outgoing call
//! Usage: cargo run --example thread_benchmark/uac_caller <caller_id>
//! Example: cargo run --example thread_benchmark/uac_caller 0

use rvoip_session_core_v2::api::simple::{SimplePeer, Config};
use tokio::time::{sleep, Duration};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get caller ID from command line args
    let args: Vec<String> = std::env::args().collect();
    let caller_id: usize = if args.len() > 1 {
        args[1].parse().unwrap_or(0)
    } else {
        0
    };

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_session_core_v2=info".parse()?)
                .add_directive("rvoip_dialog_core=info".parse()?)
                .add_directive("rvoip_media_core=info".parse()?)
        )
        .init();

    let port = 6001 + caller_id as u16;
    info!("[CALLER-{}] Starting on port {}...", caller_id, port);

    let config = Config {
        sip_port: port,
        media_port_start: 21000 + (caller_id * 100) as u16,
        media_port_end: 21100 + (caller_id * 100) as u16,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: format!("127.0.0.1:{}", port).parse()?,
        state_table_path: None,
        local_uri: format!("sip:caller{}@127.0.0.1:{}", caller_id, port),
    };

    let caller = SimplePeer::with_config(&format!("caller{}", caller_id), config).await?;

    // Wait a bit for answerer to be ready
    sleep(Duration::from_secs(2)).await;

    // Make the call
    info!("[CALLER-{}] Calling answerer...", caller_id);
    let call_id = caller.call("sip:answerer@127.0.0.1:6000").await?;
    info!("[CALLER-{}] Made call with ID: {}", caller_id, call_id);

    // Keep the call active for 20 seconds
    sleep(Duration::from_secs(20)).await;

    // Hang up
    info!("[CALLER-{}] Hanging up...", caller_id);
    caller.hangup(&call_id).await?;

    info!("[CALLER-{}] Done", caller_id);
    Ok(())
}