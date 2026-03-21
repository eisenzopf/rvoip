//! Certificate verification for DTLS
//!
//! This module implements certificate verification for DTLS.

use std::fmt;
use bytes::Bytes;
use crate::dtls::Result;

// Add crypto imports
use x509_parser::prelude::*;
use sha2::{Sha256, Digest};
use tracing::{debug, warn};

/// Certificate for DTLS connections
#[derive(Debug, Clone)]
pub struct Certificate {
    /// Raw DER-encoded X.509 certificate
    der: Bytes,
    
    /// Parsed certificate (if available)
    parsed: Option<ParsedCertificate>,
}

impl Certificate {
    /// Create a new certificate from DER-encoded data
    pub fn new(der: Bytes) -> Self {
        Self {
            der,
            parsed: None,
        }
    }
    
    /// Get the DER-encoded certificate
    pub fn der(&self) -> &Bytes {
        &self.der
    }
    
    /// Parse the certificate
    pub fn parse(&mut self) -> Result<&ParsedCertificate> {
        if self.parsed.is_none() {
            // Parse the certificate using x509-parser
            let (_, x509) = X509Certificate::from_der(&self.der)
                .map_err(|e| crate::error::Error::CertificateValidationError(
                    format!("Failed to parse certificate: {}", e)
                ))?;
            
            // Extract subject
            let subject = x509.subject().to_string();
            
            // Extract issuer
            let issuer = x509.issuer().to_string();
            
            // Extract serial number
            let serial_number = x509.serial.to_string();
            
            // Extract validity period
            let not_before = x509.validity().not_before.to_string();
            let not_after = x509.validity().not_after.to_string();
            
            // Extract subject alternative names
            let mut subject_alt_names = Vec::new();
            if let Ok(Some(ext)) = x509.subject_alternative_name() {
                for name in ext.value.general_names.iter() {
                    subject_alt_names.push(name.to_string());
                }
            }
            
            // Extract public key
            let tbs = x509.tbs_certificate;
            let public_key = match tbs.subject_pki.algorithm.algorithm.to_id_string().as_str() {
                "1.2.840.10045.2.1" => {
                    // ECDSA key
                    let curve = match tbs.subject_pki.algorithm.parameters {
                        Some(ref params) => {
                            if let Ok(oid) = params.as_oid() {
                                match oid.to_id_string().as_str() {
                                    "1.2.840.10045.3.1.7" => "P-256".to_string(),
                                    "1.3.132.0.34" => "P-384".to_string(),
                                    "1.3.132.0.35" => "P-521".to_string(),
                                    _ => format!("Unknown curve: {}", oid),
                                }
                            } else {
                                "Unknown curve".to_string()
                            }
                        },
                        None => "Unknown curve".to_string(),
                    };
                    
                    CertificatePublicKey::Ecdsa {
                        curve,
                        public_key: Bytes::copy_from_slice(&tbs.subject_pki.subject_public_key.data),
                    }
                },
                "1.2.840.113549.1.1.1" => {
                    // RSA key
                    CertificatePublicKey::Rsa {
                        modulus: Bytes::new(), // Would need to extract from ASN.1 structure
                        exponent: Bytes::new(), // Would need to extract from ASN.1 structure
                    }
                },
                _ => {
                    return Err(crate::error::Error::UnsupportedFeature(
                        format!("Unsupported public key algorithm: {}", tbs.subject_pki.algorithm.algorithm)
                    ));
                }
            };
            
            // Extract signature algorithm
            let signature_algorithm = x509.signature_algorithm.algorithm.to_string();
            
            // Extract signature
            let signature = Bytes::copy_from_slice(&x509.signature_value.data);
            
            // Create parsed certificate
            self.parsed = Some(ParsedCertificate {
                subject,
                issuer,
                serial_number,
                not_before,
                not_after,
                subject_alt_names,
                public_key,
                signature_algorithm,
                signature,
            });
        }
        
        self.parsed.as_ref()
            .ok_or_else(|| crate::error::Error::CryptoError(
                "Certificate parsing produced no result".to_string()
            ))
    }
    
    /// Get the certificate's fingerprint
    pub fn fingerprint(&mut self, algorithm: &str) -> Result<String> {
        match algorithm.to_uppercase().as_str() {
            "SHA-256" => {
                // Compute SHA-256 hash of the DER-encoded certificate
                let mut hasher = Sha256::new();
                hasher.update(&self.der);
                let result = hasher.finalize();
                
                // Format the fingerprint as a colon-separated hex string
                let fingerprint = result.iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<String>>()
                    .join(":");
                
                Ok(fingerprint)
            }
            _ => Err(crate::error::Error::UnsupportedFeature(format!("Unsupported fingerprint algorithm: {}", algorithm))),
        }
    }
}

/// Parsed X.509 certificate
#[derive(Debug, Clone)]
pub struct ParsedCertificate {
    /// Certificate subject
    pub subject: String,
    
    /// Certificate issuer
    pub issuer: String,
    
    /// Certificate serial number
    pub serial_number: String,
    
    /// Certificate not before (validity start)
    pub not_before: String,
    
    /// Certificate not after (validity end)
    pub not_after: String,
    
    /// Certificate subject alternative names
    pub subject_alt_names: Vec<String>,
    
    /// Certificate public key
    pub public_key: CertificatePublicKey,
    
    /// Certificate signature algorithm
    pub signature_algorithm: String,
    
    /// Certificate signature
    pub signature: Bytes,
}

/// Certificate public key
#[derive(Debug, Clone)]
pub enum CertificatePublicKey {
    /// RSA public key
    Rsa {
        /// Modulus
        modulus: Bytes,
        
        /// Public exponent
        exponent: Bytes,
    },
    
    /// ECDSA public key
    Ecdsa {
        /// Curve name
        curve: String,
        
        /// Raw public key
        public_key: Bytes,
    },
}

impl fmt::Display for CertificatePublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CertificatePublicKey::Rsa { .. } => write!(f, "RSA"),
            CertificatePublicKey::Ecdsa { curve, .. } => write!(f, "ECDSA ({})", curve),
        }
    }
}

/// Certificate verifier
pub trait CertificateVerifier {
    /// Verify a certificate
    fn verify(&self, cert: &Certificate, trusted_roots: &[Certificate]) -> Result<bool>;
}

/// Basic certificate verifier that checks against trusted roots
pub struct BasicCertificateVerifier;

impl CertificateVerifier for BasicCertificateVerifier {
    fn verify(&self, cert: &Certificate, trusted_roots: &[Certificate]) -> Result<bool> {
        // Parse the certificate directly from DER to get full x509 access
        let (_, x509_cert) = X509Certificate::from_der(cert.der())
            .map_err(|e| crate::error::Error::CertificateValidationError(
                format!("Failed to parse certificate for verification: {}", e)
            ))?;

        // Check validity period against current time
        if !check_validity_period(&x509_cert)? {
            return Ok(false);
        }

        // Check if the certificate is signed by a trusted root
        for root in trusted_roots {
            let (_, root_x509) = X509Certificate::from_der(root.der())
                .map_err(|e| crate::error::Error::CertificateValidationError(
                    format!("Failed to parse root certificate: {}", e)
                ))?;

            // If the issuer of the certificate matches the subject of the root,
            // then the certificate might be signed by the root
            if x509_cert.issuer() == root_x509.subject() {
                // Verify the signature using the root certificate's public key
                if let Err(e) = x509_cert.verify_signature(Some(&root_x509.tbs_certificate.subject_pki)) {
                    warn!(
                        "Certificate signature verification failed against root '{}': {}",
                        root_x509.subject(), e
                    );
                    continue;
                }

                debug!(
                    "Certificate '{}' verified against root '{}'",
                    x509_cert.subject(), root_x509.subject()
                );
                return Ok(true);
            }
        }

        // No matching root found
        warn!(
            "No trusted root found for certificate issuer '{}'",
            x509_cert.issuer()
        );
        Ok(false)
    }
}

/// Self-signed certificate verifier
///
/// Verifies that a self-signed certificate has a valid signature (signed by its own key)
/// and is within its validity period. Optionally verifies a fingerprint if provided.
pub struct SelfSignedCertificateVerifier {
    /// Optional expected fingerprint algorithm (e.g. "SHA-256")
    expected_fingerprint_algorithm: Option<String>,

    /// Optional expected fingerprint value
    expected_fingerprint: Option<String>,
}

impl SelfSignedCertificateVerifier {
    /// Create a new self-signed certificate verifier without fingerprint checking
    pub fn new() -> Self {
        Self {
            expected_fingerprint_algorithm: None,
            expected_fingerprint: None,
        }
    }

    /// Create a new self-signed certificate verifier with fingerprint checking
    pub fn with_fingerprint(algorithm: String, fingerprint: String) -> Self {
        Self {
            expected_fingerprint_algorithm: Some(algorithm),
            expected_fingerprint: Some(fingerprint),
        }
    }
}

impl CertificateVerifier for SelfSignedCertificateVerifier {
    fn verify(&self, cert: &Certificate, _trusted_roots: &[Certificate]) -> Result<bool> {
        // Parse the certificate from DER
        let (_, x509_cert) = X509Certificate::from_der(cert.der())
            .map_err(|e| crate::error::Error::CertificateValidationError(
                format!("Failed to parse self-signed certificate: {}", e)
            ))?;

        // Check validity period
        if !check_validity_period(&x509_cert)? {
            return Ok(false);
        }

        // Verify the self-signed signature: the certificate should be signed by its own public key
        if let Err(e) = x509_cert.verify_signature(Some(&x509_cert.tbs_certificate.subject_pki)) {
            warn!(
                "Self-signed certificate signature verification failed for '{}': {}",
                x509_cert.subject(), e
            );
            return Err(crate::error::Error::CertificateValidationError(
                format!("Self-signed signature verification failed: {}", e)
            ));
        }

        debug!(
            "Self-signed certificate '{}' signature verified successfully",
            x509_cert.subject()
        );

        // If a fingerprint is provided, verify it matches
        if let (Some(algorithm), Some(expected_fp)) =
            (&self.expected_fingerprint_algorithm, &self.expected_fingerprint)
        {
            let mut cert_clone = cert.clone();
            let actual_fp = cert_clone.fingerprint(algorithm)?;
            if !actual_fp.eq_ignore_ascii_case(expected_fp) {
                warn!(
                    "Fingerprint mismatch: expected '{}', got '{}'",
                    expected_fp, actual_fp
                );
                return Err(crate::error::Error::CertificateValidationError(
                    format!(
                        "Fingerprint mismatch: expected '{}', got '{}'",
                        expected_fp, actual_fp
                    )
                ));
            }
            debug!("Fingerprint verification passed");
        }

        Ok(true)
    }
}

/// Check the validity period of an X.509 certificate against the current system time.
///
/// Returns `Ok(true)` if the certificate is currently valid, `Ok(false)` if it is
/// expired or not yet valid. Returns an error only if the system time cannot be determined.
fn check_validity_period(x509_cert: &X509Certificate<'_>) -> Result<bool> {
    let validity = x509_cert.validity();

    // x509-parser's ASN1Time can be compared directly with time::OffsetDateTime
    let now = ::time::OffsetDateTime::now_utc();

    // Convert ASN1Time to a comparable form using the raw timestamp
    let not_before_ts = validity.not_before.timestamp();
    let not_after_ts = validity.not_after.timestamp();
    let now_ts = now.unix_timestamp();

    if now_ts < not_before_ts {
        warn!(
            "Certificate '{}' is not yet valid (not_before: {})",
            x509_cert.subject(),
            validity.not_before
        );
        return Ok(false);
    }

    if now_ts > not_after_ts {
        warn!(
            "Certificate '{}' has expired (not_after: {})",
            x509_cert.subject(),
            validity.not_after
        );
        return Ok(false);
    }

    Ok(true)
}

/// Certificate chain verifier
pub struct CertificateChainVerifier {
    /// Underlying certificate verifier
    verifier: Box<dyn CertificateVerifier>,
}

impl CertificateChainVerifier {
    /// Create a new certificate chain verifier
    pub fn new(verifier: Box<dyn CertificateVerifier>) -> Self {
        Self {
            verifier,
        }
    }
    
    /// Verify a certificate chain
    pub fn verify(&self, certs: &[Certificate], trusted_roots: &[Certificate]) -> Result<bool> {
        if certs.is_empty() {
            return Err(crate::error::Error::InvalidParameter("Empty certificate chain".to_string()));
        }
        
        // Verify each certificate in the chain
        for (i, cert) in certs.iter().enumerate() {
            if i == 0 {
                // For the leaf certificate, verify against the provided trusted roots
                if !self.verifier.verify(cert, trusted_roots)? {
                    return Ok(false);
                }
            } else {
                // For intermediate certificates, verify against the previous certificate
                let previous_certs = &certs[..i];
                if !self.verifier.verify(cert, previous_certs)? {
                    return Ok(false);
                }
            }
        }
        
        Ok(true)
    }
}

/// Fingerprint verifier for DTLS
pub struct FingerprintVerifier {
    /// Expected fingerprint algorithm
    algorithm: String,
    
    /// Expected fingerprint value
    fingerprint: String,
}

impl FingerprintVerifier {
    /// Create a new fingerprint verifier
    pub fn new(algorithm: String, fingerprint: String) -> Self {
        Self {
            algorithm,
            fingerprint,
        }
    }
    
    /// Verify a certificate against the expected fingerprint
    pub fn verify(&self, cert: &mut Certificate) -> Result<bool> {
        let cert_fingerprint = cert.fingerprint(&self.algorithm)?;
        Ok(cert_fingerprint.eq_ignore_ascii_case(&self.fingerprint))
    }
}

/// Generate a self-signed certificate for testing
pub fn generate_self_signed_certificate() -> Result<Certificate> {
    use rcgen::{Certificate as RcGenCertificate, CertificateParams, PKCS_ECDSA_P256_SHA256};
    
    // Create certificate parameters
    let mut params = CertificateParams::new(vec!["localhost".to_string()]);
    params.alg = &PKCS_ECDSA_P256_SHA256;
    
    // Generate the certificate
    let cert = RcGenCertificate::from_params(params)
        .map_err(|e| crate::error::Error::CertificateValidationError(
            format!("Failed to generate certificate: {}", e)
        ))?;
    
    // Get the DER-encoded certificate
    let der = cert.serialize_der()
        .map_err(|e| crate::error::Error::CertificateValidationError(
            format!("Failed to serialize certificate: {}", e)
        ))?;
    
    // Create a Certificate from the DER data
    Ok(Certificate::new(Bytes::from(der)))
}
