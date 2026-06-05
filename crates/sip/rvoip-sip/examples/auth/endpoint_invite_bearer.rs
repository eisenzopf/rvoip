//! Endpoint UAC INVITE to UnifiedCoordinator UAS with Bearer auth.
//!
//! This demonstrates the full challenged-call path:
//!
//! - `Endpoint` starts an outbound INVITE without Authorization;
//! - `UnifiedCoordinator` receives the INVITE and asks `SipAuthService`;
//! - the UAS sends a Bearer `WWW-Authenticate` challenge;
//! - the Endpoint retries with a real Bearer `Authorization` header;
//! - the UAS validates the token and answers the call.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example auth_endpoint_invite_bearer

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;
use rvoip_sip::api::AuthScheme;
use rvoip_sip::{
    BearerAuthError, BearerValidator, Config, Endpoint, EndpointProfile, Result, SessionError,
    SipAuthDecision, SipAuthScheme, SipAuthService, SipAuthSource, SipClientAuth,
    UnifiedCoordinator,
};

const UAS_PORT: u16 = 5292;
const UAC_PORT: u16 = 5293;
const TOKEN: &str = "local-dev-token";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing_if_requested();

    let uas_auth = SipAuthService::new()
        .with_bearer_validator("local-dev", Arc::new(StaticBearerValidator))
        .with_bearer_scope("sip.invite")
        .allow_bearer_over_cleartext(true);

    let uas =
        UnifiedCoordinator::new(Config::local("bearer-uas", UAS_PORT).with_signaling_only_media(9))
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
                            "[uas] accepted INVITE via {:?} subject={:?} scopes={:?}",
                            identity.scheme, identity.subject, identity.scopes
                        );
                        let call = incoming.accept().await?;
                        call.wait_for_end(Some(Duration::from_secs(10))).await?;
                        return Ok::<_, SessionError>(());
                    }
                    SipAuthDecision::Rejected { challenges } => {
                        let challenge = challenges
                            .into_iter()
                            .find(|challenge| challenge.scheme == SipAuthScheme::Bearer)
                            .ok_or_else(|| {
                                SessionError::AuthError("Bearer challenge was not generated".into())
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
        .name("bearer-uac")
        .profile(EndpointProfile::Custom(
            Config::local("bearer-uac", UAC_PORT).with_signaling_only_media(9),
        ))
        .auth(SipClientAuth::bearer_token(TOKEN).allow_bearer_over_cleartext(true))
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

struct StaticBearerValidator;

#[async_trait]
impl BearerValidator for StaticBearerValidator {
    async fn validate(
        &self,
        token: &str,
    ) -> std::result::Result<IdentityAssurance, BearerAuthError> {
        if token != TOKEN {
            return Err(BearerAuthError::Invalid("invalid token".to_string()));
        }

        let identity = IdentityId::from_string("user_alice");
        Ok(IdentityAssurance::UserAuthorized {
            identity: identity.clone(),
            user_id: identity,
            scopes: vec!["sip.invite".to_string()],
        })
    }
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
