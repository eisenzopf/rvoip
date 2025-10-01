//! Peer2 (Bob) - Receives call from Peer1 (Alice) and blind transfers to Peer3 (Charlie)

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

    println!("\n[BOB] Starting - Will receive call from Alice and transfer to Charlie...");

    // Configure Bob (Peer2)
    let config = Config {
        sip_port: 5061,
        media_port_start: 10100,
        media_port_end: 10200,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5061".parse()?,
        state_table_path: None,
        local_uri: "sip:bob@127.0.0.1:5061".to_string(),
    };

    let mut bob = SimplePeer::with_config("bob", config).await?;
    println!("[BOB] ✅ Listening on port 5061...");

    // Wait for incoming call from Alice
    println!("[BOB] ⏳ Waiting for call from Alice...");
    let incoming = bob.wait_for_call().await?;

    info!("[BOB] Incoming call from: {} (ID: {})", incoming.from, incoming.id);
    println!("[BOB] 📞 Received call from Alice!");

    // Accept the call
    println!("[BOB] 📞 Accepting call...");
    bob.accept(&incoming.id).await?;

    // Give the call time to fully establish (increased to ensure Active state)
    sleep(Duration::from_secs(4)).await;

    // Talk to Alice for a bit
    println!("[BOB] 💬 Talking to Alice...");
    sleep(Duration::from_secs(3)).await;

    // Perform blind transfer to Charlie
    println!("[BOB] 🔄 Initiating blind transfer to Charlie at sip:charlie@127.0.0.1:5062...");
    bob.transfer(&incoming.id, "sip:charlie@127.0.0.1:5062").await?;

    println!("[BOB] ✅ Transfer initiated!");
    println!("[BOB] 🔄 Alice should now be talking to Charlie");

    // After blind transfer, Bob's call should terminate
    // Wait a bit to let the transfer complete
    sleep(Duration::from_secs(2)).await;

    println!("[BOB] ✅ Test complete!");

    Ok(())
}
