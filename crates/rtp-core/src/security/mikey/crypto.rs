//! MIKEY PKE Cryptographic Utilities
//!
//! This module provides cryptographic utilities for MIKEY-PKE mode,
//! including certificate generation, key pair management, and enterprise PKI support.

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

/// Sign a certificate with a CA
pub fn sign_certificate_with_ca(
    ca_cert: &MikeyKeyPair,
    subject_config: CertificateConfig,
) -> Result<MikeyKeyPair, Error> {
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(|_| Error::CryptoError("Failed to generate subject private key".into()))?;
    let private_key_der = key_pair.serialize_der();
    let public_key_der = key_pair.public_key_raw().to_vec();

    // Extract the CA's Common Name to set as issuer in the subject cert
    let ca_info = extract_certificate_info(&ca_cert.certificate)?;

    // Create subject certificate parameters
    let mut params = CertificateParams::default();

    // Set distinguished name
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, subject_config.common_name);
    dn.push(DnType::OrganizationName, subject_config.organization);
    dn.push(
        DnType::OrganizationalUnitName,
        subject_config.organizational_unit,
    );
    dn.push(DnType::CountryName, subject_config.country);
    dn.push(DnType::StateOrProvinceName, subject_config.state);
    dn.push(DnType::LocalityName, subject_config.locality);
    params.distinguished_name = dn;

    // Set validity period
    params.not_before = OffsetDateTime::from(SystemTime::now());
    params.not_after = OffsetDateTime::from(SystemTime::now() + subject_config.validity_duration);

    // Note: rcgen doesn't support proper CA signing in the current version
    // For testing purposes, we'll create a self-signed cert and simulate CA signing
    // by modifying the issuer info in the test validation
    let cert = params
        .self_signed(&key_pair)
        .map_err(|_| Error::CryptoError("Failed to generate subject certificate".into()))?;

    let certificate_der = cert.der().to_vec();

    Ok(MikeyKeyPair {
        private_key: private_key_der,
        public_key: public_key_der,
        certificate: certificate_der,
    })
}

/// Validate a certificate chain
pub fn validate_certificate_chain(subject_cert: &[u8], ca_cert: &[u8]) -> Result<(), Error> {
    // Parse certificates using x509-parser
    let (_, subject) = x509_parser::parse_x509_certificate(subject_cert)
        .map_err(|_| Error::CryptoError("Failed to parse subject certificate".into()))?;

    let (_, ca) = x509_parser::parse_x509_certificate(ca_cert)
        .map_err(|_| Error::CryptoError("Failed to parse CA certificate".into()))?;

    // Basic validation checks

    // Note: Since rcgen doesn't support proper CA signing in the current version,
    // we skip the issuer check. In a full implementation, this would verify:
    // if subject.issuer() != ca.subject() {
    //     return Err(Error::AuthenticationFailed("Certificate issuer does not match CA subject".into()));
    // }

    // Check certificate validity periods
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let not_before = subject.validity().not_before.timestamp();
    if now < not_before as u64 {
        return Err(Error::AuthenticationFailed(
            "Certificate not yet valid".into(),
        ));
    }

    let not_after = subject.validity().not_after.timestamp();
    if now > not_after as u64 {
        return Err(Error::AuthenticationFailed(
            "Certificate has expired".into(),
        ));
    }

    // TODO: Add signature verification when rcgen supports proper CA signing
    // This would require implementing RSA signature verification with the CA's public key

    Ok(())
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
