//! Endpoint UAC INVITE to UnifiedCoordinator UAS with users-core Digest auth.
//!
//! This demonstrates the PBX-compatible challenged-call path:
//!
//! - users-core stores a user and dedicated SIP Digest HA1 credential material;
//! - `Endpoint` starts an outbound INVITE without Authorization;
//! - `UnifiedCoordinator` receives the INVITE and asks `SipAuthService`;
//! - the UAS sends a Digest `WWW-Authenticate` challenge;
//! - the Endpoint retries with a real Digest `Authorization` response;
//! - the UAS validates the HA1-backed response and answers the call.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example auth_endpoint_invite_digest

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use users_core::{
    init, CreateSipDigestCredentialRequest, CreateUserRequest, SipDigestAlgorithmFamily,
    UsersConfig, UsersCoreAuthProvider,
};

use rvoip_sip::api::AuthScheme;
use rvoip_sip::{
    Config, DigestAlgorithm, Endpoint, EndpointProfile, SessionError, SipAuthDecision,
    SipAuthScheme, SipAuthService, SipAuthSource, SipClientAuth, UnifiedCoordinator,
};

const UAS_PORT: u16 = 5294;
const UAC_PORT: u16 = 5295;
const REALM: &str = "pbx.example.com";
const SIP_USERNAME: &str = "1001";
const SIP_PASSWORD: &str = "sip-secret";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing_if_requested();

    let (_temp_dir, users) = seed_users_core().await?;
    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_digest_provider(REALM, provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256);

    let uas =
        UnifiedCoordinator::new(Config::local("digest-uas", UAS_PORT).with_signaling_only_media(9))
            .await?;
    let mut uas_events = uas.events().await?;
    let uas_task = {
        let uas = uas.clone();
        tokio::spawn(async move {
            loop {
                let Some(incoming) = uas.next_incoming_call(&mut uas_events).await? else {
                    return Err(SessionError::Other(
                        "UAS event stream closed before call completed".to_string(),
                    ));
                };

                match incoming.authenticate_with(&uas_auth).await? {
                    SipAuthDecision::Authorized(identity) => {
                        println!(
                            "[uas] accepted INVITE via {:?} username={:?}",
                            identity.scheme, identity.username
                        );
                        let call = incoming.accept().await?;
                        call.wait_for_end(Some(Duration::from_secs(10))).await?;
                        return Ok::<_, SessionError>(());
                    }
                    SipAuthDecision::Rejected { challenges } => {
                        let challenge = challenges
                            .into_iter()
                            .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
                            .ok_or_else(|| {
                                SessionError::AuthError("Digest challenge was not generated".into())
                            })?;
                        println!("[uas] challenging INVITE with {}", challenge.value);
                        incoming
                            .challenge_builder(to_auth_scheme(&challenge.scheme))
                            .with_auth_challenge(&challenge)
                            .as_proxy_challenge(challenge.source == SipAuthSource::Proxy)
                            .send()
                            .await?;
                    }
                }
            }
        })
    };

    tokio::time::sleep(Duration::from_millis(300)).await;

    let endpoint = Endpoint::builder()
        .name("digest-uac")
        .profile(EndpointProfile::Custom(
            Config::local("digest-uac", UAC_PORT).with_signaling_only_media(9),
        ))
        .auth(SipClientAuth::digest(SIP_USERNAME, SIP_PASSWORD))
        .build()
        .await?;

    let call = endpoint
        .call_and_wait(
            &format!("sip:bob@127.0.0.1:{UAS_PORT}"),
            Some(Duration::from_secs(10)),
        )
        .await?;
    println!("[uac] connected as {}", call.id());
    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    endpoint.shutdown().await?;

    uas_task
        .await
        .map_err(|err| SessionError::Other(err.to_string()))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

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
            sip_username: SIP_USERNAME.to_string(),
            realm: REALM.to_string(),
            algorithm: SipDigestAlgorithmFamily::Sha256,
            password: SIP_PASSWORD.to_string(),
        })
        .await?;

    Ok((temp_dir, users))
}

fn init_tracing_if_requested() {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
    }
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
