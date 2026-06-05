//! Custom external provider example.
//!
//! Shows the shape an application would implement when it has its own user
//! service instead of users-core, LDAP, or OIDC.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example auth_custom_provider

use std::sync::Arc;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;
use rvoip_sip::{
    CredentialAuthError, DigestAlgorithm, DigestAuth, DigestAuthenticator, DigestSecret,
    DigestSecretProvider, PasswordVerifier, SipAuthDecision, SipAuthScheme, SipAuthService,
    SipAuthSource,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = Arc::new(MyExternalUserService);
    let auth = SipAuthService::new()
        .with_basic_verifier("external", provider.clone())
        .with_digest_provider("pbx.example.com", provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256);

    let basic = BASE64_STANDARD.encode("alice:SecurePass2024");
    print_decision(
        "Basic",
        auth.authenticate_authorization(
            Some(&format!("Basic {basic}")),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            true,
        )
        .await?,
    );

    let challenge = auth
        .challenges(SipAuthSource::Origin)
        .into_iter()
        .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
        .expect("Digest challenge");
    let parsed = DigestAuthenticator::parse_challenge(&challenge.value)?;
    let computed = DigestAuth::compute_response_with_state(
        "1001",
        "sip-secret",
        &parsed,
        "REGISTER",
        "sip:pbx.example.com",
        1,
        None,
    )?;
    let digest = DigestAuth::format_authorization_with_state(
        "1001",
        &parsed,
        "sip:pbx.example.com",
        &computed,
    );
    print_decision(
        "Digest",
        auth.authenticate_authorization(
            Some(&digest),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            true,
        )
        .await?,
    );

    Ok(())
}

struct MyExternalUserService;

#[async_trait]
impl PasswordVerifier for MyExternalUserService {
    async fn verify_password(
        &self,
        username: &str,
        password: &str,
    ) -> std::result::Result<IdentityAssurance, CredentialAuthError> {
        if username == "alice" && password == "SecurePass2024" {
            Ok(identity("external:alice", ["sip.register", "sip.call"]))
        } else {
            Err(CredentialAuthError::Invalid)
        }
    }
}

#[async_trait]
impl DigestSecretProvider for MyExternalUserService {
    async fn lookup_digest_secret(
        &self,
        username: &str,
        realm: &str,
        _algorithm: DigestAlgorithm,
    ) -> std::result::Result<Option<DigestSecret>, CredentialAuthError> {
        if username == "1001" && realm == "pbx.example.com" {
            Ok(Some(DigestSecret::PlaintextPassword("sip-secret".into())))
        } else {
            Ok(None)
        }
    }
}

fn identity(id: &str, scopes: impl IntoIterator<Item = impl Into<String>>) -> IdentityAssurance {
    let identity = IdentityId::from_string(id);
    IdentityAssurance::UserAuthorized {
        identity: identity.clone(),
        user_id: identity,
        scopes: scopes.into_iter().map(Into::into).collect(),
    }
}

fn print_decision(label: &str, decision: SipAuthDecision) {
    match decision {
        SipAuthDecision::Authorized(identity) => {
            println!("{label}: authorized {identity:?}");
        }
        SipAuthDecision::Rejected { challenges } => {
            println!("{label}: rejected with {} challenge(s)", challenges.len());
        }
    }
}
