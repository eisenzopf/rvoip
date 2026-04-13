//! Register with a SIP registrar server.
//!
//! Start the registrar first:
//!   cargo run --example advanced_registrar_server
//!
//! Then run this client:
//!   cargo run --example streampeer_registration

use rvoip_session_core_v3::{Config, Registration, UnifiedCoordinator};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    let coordinator = UnifiedCoordinator::new(Config::local("alice", 5061)).await?;
    println!("Registering alice with sip:127.0.0.1:5060...");

    let handle = coordinator
        .register_with(Registration::new(
            "sip:127.0.0.1:5060",
            "alice",
            "password123",
        ))
        .await?;

    // Wait for auth flow to complete
    tokio::time::sleep(Duration::from_secs(3)).await;

    match coordinator.is_registered(&handle).await {
        Ok(true) => println!("Registered successfully!"),
        Ok(false) => println!("Registration not completed (is the server running?)"),
        Err(e) => println!("Error checking status: {}", e),
    }

    // Refresh registration
    println!("Refreshing...");
    coordinator.refresh_registration(&handle).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Unregister
    println!("Unregistering...");
    coordinator.unregister(&handle).await?;
    println!("Done.");
    Ok(())
}
