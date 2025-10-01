//! Peer3 (Charlie) - Receives the transferred call from Peer1 (Alice)

use rvoip_session_core_v2::api::simple::SimplePeer;
use rvoip_session_core_v2::api::simple::Config;
use tokio::time::{sleep, Duration};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Simple logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_session_core_v2=info".parse()?)
                .add_directive("rvoip_dialog_core=info".parse()?)
                .add_directive("rvoip_media_core=info".parse()?)
        )
        .init();

    println!("\n[CHARLIE] Starting - Will receive transferred call from Alice...");

    // Configure Charlie (Peer3)
    let config = Config {
        sip_port: 5062,
        media_port_start: 10200,
        media_port_end: 10300,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5062".parse()?,
        state_table_path: None,
        local_uri: "sip:charlie@127.0.0.1:5062".to_string(),
    };

    let mut charlie = SimplePeer::with_config("charlie", config).await?;
    println!("[CHARLIE] ✅ Listening on port 5062...");

    // Wait for the transferred call from Alice (via Bob's REFER)
    println!("[CHARLIE] ⏳ Waiting for transferred call from Alice...");
    let incoming = charlie.wait_for_call().await?;

    info!("[CHARLIE] Incoming call from: {} (ID: {})", incoming.from, incoming.id);
    println!("[CHARLIE] 📞 Received transferred call!");

    // Accept the call
    println!("[CHARLIE] 📞 Accepting call...");
    charlie.accept(&incoming.id).await?;

    // Give the call time to fully establish
    sleep(Duration::from_secs(1)).await;

    // Now talking to Alice
    println!("[CHARLIE] 💬 Now talking to Alice (post-transfer)...");
    sleep(Duration::from_secs(3)).await;

    // Wait for Alice to hang up, or hang up ourselves
    println!("[CHARLIE] ⏳ Waiting for call to end...");
    sleep(Duration::from_secs(3)).await;

    println!("[CHARLIE] ✅ Test complete!");

    Ok(())
}
