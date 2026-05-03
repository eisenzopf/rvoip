//! Register alice with the registrar, check status, and unregister.
//!
//! Run with the registrar:
//!
//!   ./examples/stream_peer/04_registration/run.sh

use std::time::Duration;

use rvoip_session_core::{Config, Registration, StreamPeer};

#[tokio::main]
async fn main() -> rvoip_session_core::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let mut peer = StreamPeer::with_config(Config::local("alice", 5061)).await?;
    println!("[alice] registering with sip:127.0.0.1:5060");

    let handle = peer
        .register_with(Registration::new(
            "sip:127.0.0.1:5060",
            "alice",
            "password123",
        ))
        .await?;

    tokio::time::sleep(Duration::from_secs(1)).await;

    if peer.is_registered(&handle).await? {
        println!("[alice] registered");
    } else {
        println!("[alice] registration is still pending");
    }

    peer.unregister(&handle).await?;
    peer.shutdown().await
}
