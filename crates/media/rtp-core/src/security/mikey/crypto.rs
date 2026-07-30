//! Retained MIKEY-PKE cryptographic utility types
//!
//! Key/certificate fixture helpers remain for compatibility and testing, but
//! the MIKEY-PKE exchange and trust path are incomplete and unavailable.

use crate::Error;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, PKCS_ECDSA_P256_SHA256};
use std::time::{Duration, SystemTime};
use time::OffsetDateTime;

/// Key pair for MIKEY-PKE operations
#[derive(Debug, Clone)]
pub struct MikeyKeyPair {
    /// Private key in PKCS#8 DER format
    pub private_key: Vec<u8>,
    /// Public key bytes in the rcgen/ring raw public-key format
    pub public_key: Vec<u8>,
    /// Certificate in X.509 DER format
    pub certificate: Vec<u8>,
}

/// Certificate configuration for enterprise environments
#[derive(Debug, Clone)]
pub struct CertificateConfig {
    /// Common Name (CN) for the certificate
    pub common_name: String,
    /// Organization (O)
    pub organization: String,
    /// Organizational Unit (OU)
    pub organizational_unit: String,
    /// Country (C)
    pub country: String,
    /// State or Province (ST)
    pub state: String,
    /// Locality (L)
    pub locality: String,
    /// Certificate validity duration
    pub validity_duration: Duration,
    /// Key size in bits
    pub key_size: usize,
}

impl Default for CertificateConfig {
    fn default() -> Self {
        Self {
            common_name: "MIKEY-PKE Entity".to_string(),
            organization: "Enterprise Communications".to_string(),
            organizational_unit: "Secure Multimedia".to_string(),
            country: "US".to_string(),
            state: "California".to_string(),
            locality: "San Francisco".to_string(),
            validity_duration: Duration::from_secs(365 * 24 * 60 * 60), // 1 year
            key_size: 2048,
        }
    }
}

impl CertificateConfig {
    /// Create configuration for enterprise server
    pub fn enterprise_server(hostname: &str) -> Self {
        Self {
            common_name: hostname.to_string(),
            organization: "Enterprise Corp".to_string(),
            organizational_unit: "Media Server".to_string(),
            country: "US".to_string(),
            state: "California".to_string(),
            locality: "San Francisco".to_string(),
            validity_duration: Duration::from_secs(2 * 365 * 24 * 60 * 60), // 2 years
            key_size: 2048,
        }
    }

    /// Create configuration for enterprise client
    pub fn enterprise_client(user_id: &str) -> Self {
        Self {
            common_name: format!("User {}", user_id),
            organization: "Enterprise Corp".to_string(),
            organizational_unit: "Media Client".to_string(),
            country: "US".to_string(),
            state: "California".to_string(),
            locality: "San Francisco".to_string(),
            validity_duration: Duration::from_secs(365 * 24 * 60 * 60), // 1 year
            key_size: 2048,
        }
    }

    /// Create configuration for high-security environments
    pub fn high_security(entity_name: &str) -> Self {
        Self {
            common_name: entity_name.to_string(),
            organization: "Secure Communications Inc".to_string(),
            organizational_unit: "High Security Division".to_string(),
            country: "US".to_string(),
            state: "Virginia".to_string(),
            locality: "Washington DC".to_string(),
            validity_duration: Duration::from_secs(90 * 24 * 60 * 60), // 90 days
            key_size: 4096,                                            // Higher security
        }
    }
}

/// Generate a new key pair and certificate for MIKEY-PKE.
///
/// The beta dependency graph intentionally avoids the no-fixed `rsa`
/// crate. This helper now emits P-256 test credentials for the existing
/// MIKEY-PKE demo path; the PKE payload encryption remains a placeholder
/// and is not a beta security claim.
pub fn generate_key_pair_and_certificate(config: CertificateConfig) -> Result<MikeyKeyPair, Error> {
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(|_| Error::CryptoError("Failed to generate private key".into()))?;
    let private_key_der = key_pair.serialize_der();
    let public_key_der = key_pair.public_key_raw().to_vec();

    // Create certificate parameters
    let mut params = CertificateParams::default();

    // Set distinguished name
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, config.common_name);
    dn.push(DnType::OrganizationName, config.organization);
    dn.push(DnType::OrganizationalUnitName, config.organizational_unit);
    dn.push(DnType::CountryName, config.country);
    dn.push(DnType::StateOrProvinceName, config.state);
    dn.push(DnType::LocalityName, config.locality);
    params.distinguished_name = dn;

    // Set validity period (convert SystemTime to OffsetDateTime)
    params.not_before = OffsetDateTime::from(SystemTime::now());
    params.not_after = OffsetDateTime::from(SystemTime::now() + config.validity_duration);

    // Generate certificate
    let cert = params
        .self_signed(&key_pair)
        .map_err(|_| Error::CryptoError("Failed to generate certificate".into()))?;

    let certificate_der = cert.der().to_vec();

    Ok(MikeyKeyPair {
        private_key: private_key_der,
        public_key: public_key_der,
        certificate: certificate_der,
    })
}

/// Generate a CA (Certificate Authority) certificate and key pair
pub fn generate_ca_certificate(config: CertificateConfig) -> Result<MikeyKeyPair, Error> {
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(|_| Error::CryptoError("Failed to generate CA private key".into()))?;
    let private_key_der = key_pair.serialize_der();
    let public_key_der = key_pair.public_key_raw().to_vec();

    // Create CA certificate parameters
    let mut params = CertificateParams::default();

    // Set distinguished name for CA
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, format!("{} CA", config.common_name));
    dn.push(DnType::OrganizationName, config.organization);
    dn.push(
        DnType::OrganizationalUnitName,
        "Certificate Authority".to_string(),
    );
    dn.push(DnType::CountryName, config.country);
    dn.push(DnType::StateOrProvinceName, config.state);
    dn.push(DnType::LocalityName, config.locality);
    params.distinguished_name = dn;

    // Set validity period (CA typically has longer validity)
    params.not_before = OffsetDateTime::from(SystemTime::now());
    params.not_after = OffsetDateTime::from(SystemTime::now() + config.validity_duration * 2);

    // Make it a CA certificate
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);

    // Generate CA certificate
    let cert = params
        .self_signed(&key_pair)
        .map_err(|_| Error::CryptoError("Failed to generate CA certificate".into()))?;

    let certificate_der = cert.der().to_vec();

    Ok(MikeyKeyPair {
        private_key: private_key_der,
        public_key: public_key_der,
        certificate: certificate_der,
    })
}

/// Sign a certificate with a CA.
///
/// This public helper is retained for source compatibility, but MIKEY-PKE CA
/// signing is not implemented in this release. It always fails closed rather
/// than returning the self-signed certificate produced by the old placeholder.
pub fn sign_certificate_with_ca(
    _ca_cert: &MikeyKeyPair,
    _subject_config: CertificateConfig,
) -> Result<MikeyKeyPair, Error> {
    Err(Error::UnsupportedFeature(
        "MIKEY-PKE CA certificate signing is not implemented".to_string(),
    ))
}

/// Validate a certificate chain.
///
/// This public helper is retained for source compatibility, but issuer and
/// signature validation are not implemented in this release. It always fails
/// closed instead of treating parseability and validity dates as trust.
pub fn validate_certificate_chain(_subject_cert: &[u8], _ca_cert: &[u8]) -> Result<(), Error> {
    Err(Error::UnsupportedFeature(
        "MIKEY-PKE certificate-chain validation is not implemented".to_string(),
    ))
}

/// Extract certificate information for display/logging
pub fn extract_certificate_info(cert_der: &[u8]) -> Result<CertificateInfo, Error> {
    let (_, cert) = x509_parser::parse_x509_certificate(cert_der)
        .map_err(|_| Error::CryptoError("Failed to parse certificate".into()))?;

    let subject = cert.subject();
    let issuer = cert.issuer();

    Ok(CertificateInfo {
        subject_cn: extract_cn_from_name(subject),
        issuer_cn: extract_cn_from_name(issuer),
        serial_number: format!("{:?}", cert.serial),
        not_before: cert.validity().not_before.timestamp(),
        not_after: cert.validity().not_after.timestamp(),
    })
}

/// Certificate information for display
#[derive(Debug, Clone)]
pub struct CertificateInfo {
    /// Subject Common Name
    pub subject_cn: String,
    /// Issuer Common Name
    pub issuer_cn: String,
    /// Serial number
    pub serial_number: String,
    /// Not valid before (Unix timestamp)
    pub not_before: i64,
    /// Not valid after (Unix timestamp)
    pub not_after: i64,
}

/// Extract Common Name from X.509 Name
fn extract_cn_from_name(name: &x509_parser::x509::X509Name) -> String {
    for rdn in name.iter() {
        for attr in rdn.iter() {
            if let Ok(cn) = attr.attr_value().as_str() {
                if attr.attr_type() == &x509_parser::oid_registry::OID_X509_COMMON_NAME {
                    return cn.to_string();
                }
            }
        }
    }
    "Unknown".to_string()
}
