//! WebAuthn/passkey users-core example.
//!
//! This starts a passkey registration ceremony and prints the browser
//! challenge JSON. A complete browser demo should send the challenge to
//! `navigator.credentials.create(...)` and pass the response to
//! `finish_registration(...)`.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example auth_webauthn_passkeys

use std::sync::Arc;

use rvoip_webauthn::{WebauthnConfig, WebauthnService};
use tempfile::TempDir;
use url::Url;
use users_core::{init, CreateUserRequest, UsersConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let users = init(UsersConfig {
        database_url: format!(
            "sqlite://{}?mode=rwc",
            temp_dir.path().join("users.db").display()
        ),
        ..UsersConfig::default()
    })
    .await?;
    let user = users
        .create_user(CreateUserRequest {
            username: "alice".to_string(),
            password: "SecurePass2026".to_string(),
            email: Some("alice@example.test".to_string()),
            display_name: Some("Alice Example".to_string()),
            roles: vec!["user".to_string()],
        })
        .await?;

    let webauthn = WebauthnService::new(
        WebauthnConfig::new(
            "localhost",
            Url::parse("http://localhost:8080")?,
            "RVoIP Example",
        ),
        Arc::new(users),
    )?;
    let registration = webauthn.start_registration(&user.id).await?;
    println!("passkey ceremony id: {}", registration.ceremony_id);
    println!("{}", serde_json::to_string_pretty(&registration.challenge)?);
    Ok(())
}
