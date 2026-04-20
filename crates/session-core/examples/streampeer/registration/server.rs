//! SIP registrar server with test user alice/password123.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_registration_server
//! Or with client:  ./examples/streampeer/registration/run.sh

use rvoip_session_core::{Config, UnifiedCoordinator};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let coordinator = UnifiedCoordinator::new(Config::local("registrar", 5060)).await?;
    let mut users = HashMap::new();
    users.insert("alice".to_string(), "password123".to_string());
    let _registrar = coordinator
        .start_registration_server("test.local", users)
        .await?;

    println!("Registrar running on port 5060 (user: alice/password123)");
    println!("Press Ctrl+C to stop.");

    tokio::signal::ctrl_c().await?;
    println!("\nShutting down.");

    std::process::exit(0);
}
