//! Register alice with the registrar, check status, refresh, and unregister.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_registration_client
//! Or with server:  ./examples/streampeer/registration/run.sh

use rvoip_session_core::{Config, Registration, UnifiedCoordinator};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
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

    sleep(Duration::from_secs(3)).await;

    match coordinator.is_registered(&handle).await {
        Ok(true) => println!("Registered successfully!"),
        Ok(false) => println!("Registration not completed."),
        Err(e) => println!("Error checking status: {}", e),
    }

    println!("Refreshing...");
    coordinator.refresh_registration(&handle).await?;
    sleep(Duration::from_secs(2)).await;

    println!("Unregistering...");
    coordinator.unregister(&handle).await?;
    println!("Done.");

    std::process::exit(0);
}
