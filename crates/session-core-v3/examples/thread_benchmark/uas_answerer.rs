//! UAS Answerer - accepts incoming calls on port 6000

use rvoip_session_core_v3::{StreamPeer, Config};
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_session_core_v3=info".parse()?)
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

    let mut answerer = StreamPeer::with_config(config).await?;
    info!("[ANSWERER] Ready to accept calls");

    // Accept calls for 30 seconds
    let mut active_handles = Vec::new();
    let start = std::time::Instant::now();

    while start.elapsed() < Duration::from_secs(30) {
        // Wait for incoming call with a short timeout
        match tokio::time::timeout(Duration::from_millis(100), answerer.wait_for_incoming()).await {
            Ok(Ok(incoming)) => {
                info!("[ANSWERER] Incoming call from {} with ID {}", incoming.from, incoming.call_id);

                // Accept the call
                match incoming.accept().await {
                    Ok(handle) => {
                        info!("[ANSWERER] Accepted call {}", handle.id());
                        active_handles.push(handle);
                    }
                    Err(e) => {
                        warn!("[ANSWERER] Error accepting call: {}", e);
                    }
                }
            }
            Ok(Err(e)) => {
                warn!("[ANSWERER] Error waiting for call: {}", e);
                break;
            }
            Err(_) => {
                // Timeout, no incoming call - continue
            }
        }
    }

    info!("[ANSWERER] Handled {} calls", active_handles.len());

    // Hang up all active calls
    for handle in &active_handles {
        if let Err(e) = handle.hangup().await {
            warn!("[ANSWERER] Error hanging up call {}: {}", handle.id(), e);
        }
    }

    info!("[ANSWERER] Shutting down");
    Ok(())
}
