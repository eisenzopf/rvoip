//! users-core backed SIP authentication example.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example auth_users_core_service

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use tempfile::TempDir;
use users_core::{
    init, CreateSipDigestCredentialRequest, CreateUserRequest, SipDigestAlgorithmFamily,
    UsersConfig, UsersCoreAuthProvider,
};

use rvoip_sip::{
    AuthIdentity, Config, DigestAuth, DigestAuthenticator, Endpoint, EndpointProfile,
    SipAuthDecision, SipAuthScheme, SipAuthService, SipAuthSource, SipClientAuth,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let db_url = format!(
        "sqlite://{}?mode=rwc",
        temp_dir.path().join("users.db").display()
    );
    let users = init(UsersConfig {
        database_url: db_url,
        ..UsersConfig::default()
    })
    .await?;

    let user = users
        .create_user(CreateUserRequest {
            username: "alice".to_string(),
            password: "SecurePass2024".to_string(),
            email: Some("alice@example.com".to_string()),
            display_name: Some("Alice".to_string()),
            roles: vec!["user".to_string()],
        })
        .await?;
    users
        .create_sip_digest_credential(CreateSipDigestCredentialRequest {
            user_id: user.id.clone(),
            sip_username: "1001".to_string(),
            realm: "pbx.example.com".to_string(),
            algorithm: SipDigestAlgorithmFamily::Sha256,
            password: "sip-secret".to_string(),
        })
        .await?;

    let login = users
        .authenticate_password("alice", "SecurePass2024")
        .await?;

    let uac = Endpoint::builder()
        .name("auth-uac")
        .profile(EndpointProfile::Custom(Config::local("auth-uac", 0)))
        .auth(SipClientAuth::bearer_token(login.access_token.clone()))
        .build()
        .await?;

    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_bearer_validator("users-core", provider.clone())
        .with_basic_verifier("users-core", provider.clone())
        .allow_basic_over_cleartext(true)
        .with_digest_provider("pbx.example.com", provider)
        .with_digest_provider_algorithm(rvoip_sip::DigestAlgorithm::SHA256);

    let bearer = uas_auth
        .authenticate_authorization(
            Some(&format!("Bearer {}", login.access_token)),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            true,
        )
        .await?;
    print_identity("Bearer", bearer);

    let basic_token = BASE64_STANDARD.encode("alice:SecurePass2024");
    let basic = uas_auth
        .authenticate_authorization(
            Some(&format!("Basic {basic_token}")),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            false,
        )
        .await?;
    print_identity("Basic", basic);

    let digest_challenge = uas_auth
        .challenges(SipAuthSource::Origin)
        .into_iter()
        .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
        .expect("Digest challenge should be enabled");
    let parsed_challenge = DigestAuthenticator::parse_challenge(&digest_challenge.value)?;
    let computed = DigestAuth::compute_response_with_state(
        "1001",
        "sip-secret",
        &parsed_challenge,
        "REGISTER",
        "sip:pbx.example.com",
        1,
        None,
    )?;
    let digest_header = DigestAuth::format_authorization_with_state(
        "1001",
        &parsed_challenge,
        "sip:pbx.example.com",
        &computed,
    );
    let digest = uas_auth
        .authenticate_authorization(
            Some(&digest_header),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            true,
        )
        .await?;
    print_identity("Digest", digest);

    uac.shutdown().await?;
    Ok(())
}

fn print_identity(label: &str, decision: SipAuthDecision) {
    match decision {
        SipAuthDecision::Authorized(AuthIdentity {
            scheme,
            username,
            subject,
            scopes,
            ..
        }) => {
            println!(
                "{label}: {scheme:?} username={username:?} subject={subject:?} scopes={scopes:?}"
            );
        }
        SipAuthDecision::Rejected { challenges } => {
            println!("{label}: rejected with {} challenges", challenges.len());
        }
    }
}
