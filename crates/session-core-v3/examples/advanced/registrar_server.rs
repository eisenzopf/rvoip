//! SIP registrar server with digest authentication.
//!
//!   cargo run --example advanced_registrar_server
//!
//! Starts a registration server on port 5060 with two test users:
//!   alice / password123
//!   bob   / secret456
//!
//! Test with: cargo run --example streampeer_registration

use rvoip_session_core_v3::{Config, UnifiedCoordinator};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let coordinator = UnifiedCoordinator::new(Config::local("registrar", 5060)).await?;

    let mut users = HashMap::new();
    users.insert("alice".to_string(), "password123".to_string());
    users.insert("bob".to_string(), "secret456".to_string());

    let _registrar = coordinator
        .start_registration_server("test.local", users)
        .await?;

    println!("Registrar server on port 5060");
    println!("  Users: alice/password123, bob/secret456");
    println!("  Realm: test.local");
    println!();
    println!("Test with: cargo run --example streampeer_registration");
    println!("Press Ctrl+C to stop.");

    tokio::signal::ctrl_c().await?;
    println!("Shutting down.");
    Ok(())
}
