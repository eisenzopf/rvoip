//! UAS Answerer - accepts incoming calls on port 6000

use rvoip_session_core_v2::api::simple::{SimplePeer, Config};
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_session_core_v2=info".parse()?)
                .add_directive("rvoip_dialog_core=info".parse()?)
                .add_directive("rvoip_media_core=info".parse()?)
        )
        .init();

    info!("[ANSWERER] Starting on port 6000...");

    let config = Config {
        sip_port: 6000,
        media_port_start: 20000,
        media_port_end: 20100,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:6000".parse()?,
        state_table_path: None,
        local_uri: "sip:answerer@127.0.0.1:6000".to_string(),
    };

    let mut answerer = SimplePeer::with_config("answerer", config).await?;
    info!("[ANSWERER] Ready to accept calls");

    // Accept calls for 30 seconds
    let mut active_calls = Vec::new();
    let start = std::time::Instant::now();

    while start.elapsed() < Duration::from_secs(30) {
        // Check for incoming calls (non-blocking)
        if let Some(call) = answerer.incoming_call().await {
            info!("[ANSWERER] Incoming call from {} with ID {}", call.from, call.id);

            // Accept the call
            answerer.accept(&call.id).await?;
            info!("[ANSWERER] Accepted call {}", call.id);
            active_calls.push(call.id);
        }

        // Keep the loop responsive
        sleep(Duration::from_millis(100)).await;
    }

    info!("[ANSWERER] Handled {} calls", active_calls.len());

    // Hang up all active calls
    for call_id in active_calls {
        if let Err(e) = answerer.hangup(&call_id).await {
            warn!("[ANSWERER] Error hanging up call {}: {}", call_id, e);
        }
    }

    info!("[ANSWERER] Shutting down");
    Ok(())
}