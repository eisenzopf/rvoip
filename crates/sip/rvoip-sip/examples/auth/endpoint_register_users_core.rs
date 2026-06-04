//! Endpoint UAC REGISTER to UnifiedCoordinator UAS with users-core auth.
//!
//! This is the real local SIP flow developers usually need to see:
//!
//! - users-core stores a user and dedicated SIP Digest HA1 credential material;
//! - `Endpoint` acts as the UAC and registers with a SIP account;
//! - `UnifiedCoordinator` acts as the UAS/registrar application surface;
//! - inbound REGISTERs are authenticated through `SipAuthService` backed by
//!   `UsersCoreAuthProvider`;
//! - the first REGISTER gets a 401 Digest challenge and the Endpoint retries
//!   with a real Digest `Authorization` response.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example auth_endpoint_register_users_core

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use users_core::{
    init, CreateSipDigestCredentialRequest, CreateUserRequest, SipDigestAlgorithmFamily,
    UsersConfig, UsersCoreAuthProvider,
};

use rvoip_sip::api::AuthScheme;
use rvoip_sip::{
    Config, Endpoint, EndpointProfile, EndpointRegistrationStatus, Event, SessionError, SipAccount,
    SipAuthDecision, SipAuthScheme, SipAuthService, SipAuthSource, UnifiedCoordinator,
};

const REGISTRAR_PORT: u16 = 5290;
const REALM: &str = "pbx.example.com";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (_temp_dir, users) = seed_users_core().await?;
    let users = Arc::new(users);
    let provider = UsersCoreAuthProvider::shared(users);
    let auth = SipAuthService::new()
        .with_digest_provider(REALM, provider)
        .with_digest_provider_algorithm(rvoip_sip::DigestAlgorithm::SHA256);

    let registrar =
        UnifiedCoordinator::new(Config::local("users-core-registrar", REGISTRAR_PORT)).await?;
    let mut registrar_events = registrar.events().await?;
    let registrar_task = {
        let auth = auth.clone();
        tokio::spawn(async move {
            while let Some(event) = registrar_events.next().await {
                let Event::IncomingRegister { register } = event else {
                    continue;
                };

                match register.authenticate_with(&auth).await? {
                    SipAuthDecision::Authorized(identity) => {
                        println!(
                            "[uas] accepted REGISTER via {:?} username={:?}",
                            identity.scheme, identity.username
                        );
                        register.accept_builder().with_expires(300).send().await?;
                        return Ok::<_, SessionError>(());
                    }
                    SipAuthDecision::Rejected { challenges } => {
                        let challenge = challenges
                            .into_iter()
                            .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
                            .ok_or_else(|| {
                                SessionError::AuthError("Digest challenge was not generated".into())
                            })?;
                        println!("[uas] challenging REGISTER with {}", challenge.value);
                        register
                            .challenge_builder(to_auth_scheme(&challenge.scheme))
                            .with_raw_www_authenticate(challenge.value)
                            .as_proxy_challenge(challenge.source == SipAuthSource::Proxy)
                            .send()
                            .await?;
                    }
                }
            }
            Err(SessionError::Other(
                "registrar event stream closed before REGISTER completed".to_string(),
            ))
        })
    };

    tokio::time::sleep(Duration::from_millis(300)).await;

    let account = SipAccount::new(
        format!("sip:127.0.0.1:{REGISTRAR_PORT}"),
        "1001",
        "sip-secret",
    )
    .auth_username("1001")
    .expires(300);

    let mut endpoint = Endpoint::builder()
        .name("endpoint-uac")
        .profile(EndpointProfile::Custom(Config::local("endpoint-uac", 5291)))
        .sip_account(account)
        .build()
        .await?;

    let registration = endpoint
        .register_and_wait(Some(Duration::from_secs(10)))
        .await?;
    println!("[uac] registration result: {:?}", registration.status);
    if registration.status != EndpointRegistrationStatus::Registered {
        return Err(anyhow::anyhow!(
            "expected registration to succeed, got {:?}",
            registration.status
        ));
    }

    endpoint.shutdown().await?;
    registrar
        .shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;
    registrar_task
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))??;

    Ok(())
}

async fn seed_users_core() -> anyhow::Result<(TempDir, users_core::AuthenticationService)> {
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
            user_id: user.id,
            sip_username: "1001".to_string(),
            realm: REALM.to_string(),
            algorithm: SipDigestAlgorithmFamily::Sha256,
            password: "sip-secret".to_string(),
        })
        .await?;

    Ok((temp_dir, users))
}

fn to_auth_scheme(scheme: &SipAuthScheme) -> AuthScheme {
    match scheme {
        SipAuthScheme::Digest => AuthScheme::Digest,
        SipAuthScheme::Bearer => AuthScheme::Bearer,
        SipAuthScheme::Basic => AuthScheme::Basic,
        SipAuthScheme::Aka => AuthScheme::Aka,
        SipAuthScheme::Other(_) => AuthScheme::Digest,
        _ => AuthScheme::Digest,
    }
}
