//! IMS AKA provider-shape example.
//!
//! SIP AKA is carried in SIP auth headers as a Digest-family challenge. This
//! example uses a deterministic lab vector. Production systems should source
//! vectors from SIM/USIM, HSS/AuC, UDM/AUSF, or a broker-backed provider.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example auth_ims_aka_provider

use rvoip_ims_aka::{ImsAkaAlgorithm, ImsAkaVector, StaticAkaProvider};
use rvoip_sip::{AkaClientProvider, AkaVectorProvider, SipAuthSource};

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    let provider = StaticAkaProvider::new(ImsAkaVector {
        username: "sip-user".to_string(),
        realm: "ims.example.test".to_string(),
        nonce: "base64-rand-autn".to_string(),
        algorithm: ImsAkaAlgorithm::AKAv1Md5,
        expected_response: "expected-res".to_string(),
        subject: Some("imsi-001010123456789".to_string()),
        scopes: vec!["sip.register".to_string()],
    });

    let challenge = provider.challenge(SipAuthSource::Origin);
    println!("WWW-Authenticate: {}", challenge.value);

    let authorization =
        provider.authorization(&challenge.value, "REGISTER", "sip:ims.example.test", 1)?;
    println!("Authorization: {authorization}");

    let identity = provider
        .validate(&authorization, "REGISTER", "sip:ims.example.test", None)
        .await?
        .expect("valid AKA response");
    println!("AKA identity: {identity:?}");
    Ok(())
}
