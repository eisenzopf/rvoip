//! Peer1 (Alice) - Makes a call to Peer2 (Bob) and gets transferred to Peer3 (Charlie)

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

    println!("\n[ALICE] Starting - Will call Bob and be transferred to Charlie...");

    // Configure Alice (Peer1)
    let config = Config {
        sip_port: 5060,
        media_port_start: 10000,
        media_port_end: 10100,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5060".parse()?,
        state_table_path: None,
        local_uri: "sip:alice@127.0.0.1:5060".to_string(),
    };

    let alice = SimplePeer::with_config("alice", config).await?;

    // Give peers time to start
    println!("[ALICE] Waiting for other peers to start...");
    sleep(Duration::from_secs(3)).await;

    // Make the call to Bob (Peer2)
    println!("[ALICE] 📞 Calling Bob at sip:bob@127.0.0.1:5061...");
    let call_id = alice.call("sip:bob@127.0.0.1:5061").await?;
    info!("[ALICE] Call established with ID: {:?}", call_id);

    // Talk to Bob for a bit
    println!("[ALICE] 💬 Talking to Bob...");
    sleep(Duration::from_secs(3)).await;

    // Wait for transfer to happen (initiated by Bob)
    println!("[ALICE] ⏳ Waiting for Bob to transfer me to Charlie...");
    sleep(Duration::from_secs(5)).await;

    // After transfer, we should be talking to Charlie
    println!("[ALICE] 💬 Now talking to Charlie (post-transfer)...");
    sleep(Duration::from_secs(3)).await;

    // Hangup
    println!("[ALICE] 📴 Hanging up...");
    alice.hangup(&call_id).await?;

    println!("[ALICE] ✅ Test complete!");
    sleep(Duration::from_secs(1)).await;

    Ok(())
}
