//! Optional LDAP-backed Basic-over-TLS provider example.
//!
//! Run with:
//!
//!   RVOIP_LDAP_URL=ldap://127.0.0.1:1389 \
//!   RVOIP_LDAP_BIND_DN='cn=admin,dc=rvoip,dc=local' \
//!   RVOIP_LDAP_BIND_PASSWORD=adminpassword \
//!   RVOIP_LDAP_USER_BASE_DN='ou=users,dc=rvoip,dc=local' \
//!     cargo run -p rvoip-sip --example auth_ldap_basic_provider

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rvoip_ldap::{LdapPasswordVerifier, LdapPasswordVerifierConfig};
use rvoip_sip::{SipAuthDecision, SipAuthService, SipAuthSource};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(url) = std::env::var("RVOIP_LDAP_URL").ok() else {
        println!("Skipping LDAP example; set RVOIP_LDAP_URL.");
        println!("Local fixture hint: cd ~/Developer/openldap && docker compose up -d");
        return Ok(());
    };

    let user_base_dn = std::env::var("RVOIP_LDAP_USER_BASE_DN")
        .unwrap_or_else(|_| "ou=users,dc=rvoip,dc=local".to_string());
    let mut config = LdapPasswordVerifierConfig::new(url, user_base_dn)
        .with_scopes(["sip.register", "sip.call"]);
    if let (Ok(bind_dn), Ok(bind_password)) = (
        std::env::var("RVOIP_LDAP_BIND_DN"),
        std::env::var("RVOIP_LDAP_BIND_PASSWORD"),
    ) {
        config = config.with_bind_credentials(bind_dn, bind_password);
    }

    let username =
        std::env::var("RVOIP_LDAP_TEST_USERNAME").unwrap_or_else(|_| "alice".to_string());
    let password =
        std::env::var("RVOIP_LDAP_TEST_PASSWORD").unwrap_or_else(|_| "alicepass".to_string());
    let verifier = Arc::new(LdapPasswordVerifier::new(config)?);
    let auth = SipAuthService::new().with_basic_verifier("ldap", verifier);

    let basic = BASE64_STANDARD.encode(format!("{username}:{password}"));
    match auth
        .authenticate_authorization(
            Some(&format!("Basic {basic}")),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            true,
        )
        .await?
    {
        SipAuthDecision::Authorized(identity) => {
            println!("LDAP Basic authorized username={:?}", identity.username);
        }
        SipAuthDecision::Rejected { challenges } => {
            println!("LDAP Basic rejected with {} challenge(s)", challenges.len());
        }
    }

    Ok(())
}
