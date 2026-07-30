//! MIKEY-PKE fail-closed availability example.
//!
//! rvoip 0.3.5 retains the MIKEY-PKE public configuration and certificate
//! utility types for compatibility, but it does not advertise or negotiate
//! MIKEY-PKE. This example demonstrates the typed unsupported results without
//! implying that CA signing, chain validation, or PKE key exchange succeeded.

use rvoip_rtp_core::{
    api::common::{SecurityConfig, SecurityError},
    security::mikey::{
        crypto::{
            extract_certificate_info, generate_ca_certificate, sign_certificate_with_ca,
            validate_certificate_chain, CertificateConfig,
        },
        Mikey, MikeyConfig, MikeyKeyExchangeMethod, MikeyRole,
    },
    Error,
};
use tracing::info;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Real self-signed key/certificate generation and metadata extraction are
    // available independently of the incomplete PKE trust path.
    let ca = generate_ca_certificate(CertificateConfig::high_security("Example Root"))?;
    let ca_info = extract_certificate_info(&ca.certificate)?;
    info!(subject = %ca_info.subject_cn, "generated a self-signed CA certificate");

    let signing = sign_certificate_with_ca(
        &ca,
        CertificateConfig::enterprise_client("alice@example.com"),
    );
    assert!(matches!(signing, Err(Error::UnsupportedFeature(_))));

    let validation = validate_certificate_chain(&ca.certificate, &ca.certificate);
    assert!(matches!(validation, Err(Error::UnsupportedFeature(_))));

    let mikey = Mikey::try_new(
        MikeyConfig {
            method: MikeyKeyExchangeMethod::Pk,
            certificate: Some(ca.certificate.clone()),
            private_key: Some(ca.private_key.clone()),
            peer_certificate: Some(ca.certificate.clone()),
            ..Default::default()
        },
        MikeyRole::Initiator,
    );
    assert!(matches!(mikey, Err(Error::UnsupportedFeature(_))));

    let config = SecurityConfig::mikey_pke_with_certificates(
        ca.certificate.clone(),
        ca.private_key.clone(),
        ca.certificate,
    );
    assert!(matches!(
        config.validate(),
        Err(SecurityError::UnsupportedFeature(_))
    ));

    info!("MIKEY correctly failed closed; use SDES or explicitly provisioned direct SRTP");
    Ok(())
}
