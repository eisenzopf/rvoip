//! Register with a SIP registrar server.
//!
//!   cargo run --example streampeer_registration

use rvoip_session_core_v3::{Config, Registration, UnifiedCoordinator};
use std::collections::HashMap;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    // --- Background: registrar server with test users ---
    tokio::spawn(async {
        let coordinator = UnifiedCoordinator::new(Config::local("registrar", 5060)).await.unwrap();
        let mut users = HashMap::new();
        users.insert("alice".to_string(), "password123".to_string());
        let _registrar = coordinator
            .start_registration_server("test.local", users)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;
    });
    tokio::time::sleep(Duration::from_secs(1)).await;

    // --- Demo: register with the server ---
    let coordinator = UnifiedCoordinator::new(Config::local("alice", 5061)).await?;
    println!("Registering alice with sip:127.0.0.1:5060...");

    let handle = coordinator
        .register_with(Registration::new("sip:127.0.0.1:5060", "alice", "password123"))
        .await?;

    tokio::time::sleep(Duration::from_secs(3)).await;

    match coordinator.is_registered(&handle).await {
        Ok(true) => println!("Registered successfully!"),
        Ok(false) => println!("Registration not completed."),
        Err(e) => println!("Error checking status: {}", e),
    }

    println!("Refreshing...");
    coordinator.refresh_registration(&handle).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    println!("Unregistering...");
    coordinator.unregister(&handle).await?;
    println!("Done.");
    Ok(())
}
