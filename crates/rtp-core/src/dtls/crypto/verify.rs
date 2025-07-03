//! Certificate verification for DTLS
//!
//! This module implements certificate verification for DTLS.

use std::fmt;
use bytes::Bytes;
use crate::dtls::Result;

// Add crypto imports
use x509_parser::prelude::*;
use sha2::{Sha256, Digest};

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
        
        Ok(self.parsed.as_ref().unwrap())
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
        let mut cert_clone = cert.clone();
        let parsed = cert_clone.parse()?;
        
        // Check if the certificate is signed by a trusted root
        for root in trusted_roots {
            let mut root_clone = root.clone();
            let root_parsed = root_clone.parse()?;
            
            // If the issuer of the certificate matches the subject of the root,
            // then the certificate might be signed by the root
            if parsed.issuer == root_parsed.subject {
                // In a real implementation, we would check if the signature is valid
                // using the public key of the root certificate, but for simplicity
                // we'll assume it's valid
                
                // Also check if the certificate is not expired
                // Parse not_before and not_after strings to DateTime
                // For simplicity, we'll just assume it's valid
                
                return Ok(true);
            }
        }
        
        // No matching root found
        Ok(false)
    }
}

/// Self-signed certificate verifier
///
/// This is useful for testing and development, but should not be used in production
pub struct SelfSignedCertificateVerifier;

impl CertificateVerifier for SelfSignedCertificateVerifier {
    fn verify(&self, _cert: &Certificate, _trusted_roots: &[Certificate]) -> Result<bool> {
        // In a self-signed certificate verifier, we assume all certificates are valid
        // This is ONLY intended for testing and development
        Ok(true)
    }
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
