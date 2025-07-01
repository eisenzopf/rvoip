//! Fingerprint generation and verification
//!
//! This module provides functions for generating and verifying fingerprints.

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::api::common::error::SecurityError;
use crate::dtls::DtlsConnection;

/// Generate a fingerprint from a DTLS connection
pub async fn generate_fingerprint(
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<String, SecurityError> {
    let conn_guard = connection.lock().await;
    
    if let Some(conn) = conn_guard.as_ref() {
        // Get the certificate from the connection
        if let Some(cert) = conn.local_certificate() {
            // Create a mutable copy of the certificate to compute fingerprint
            let mut cert_copy = cert.clone();
            match cert_copy.fingerprint("SHA-256") {
                Ok(fingerprint) => Ok(fingerprint),
                Err(e) => Err(SecurityError::Internal(format!("Failed to get fingerprint: {}", e))),
            }
        } else {
            Err(SecurityError::Configuration("No certificate available".to_string()))
        }
    } else {
        Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
    }
}

/// Verify a remote fingerprint
pub async fn verify_fingerprint(
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
    remote_fingerprint: &str,
    remote_fingerprint_algorithm: &str,
) -> Result<bool, SecurityError> {
    let conn_guard = connection.lock().await;
    
    if let Some(conn) = conn_guard.as_ref() {
        // Get the remote certificate from the connection
        if let Some(cert) = conn.remote_certificate() {
            // Create a mutable copy of the certificate to compute fingerprint
            let mut cert_copy = cert.clone();
            
            // Determine the algorithm to use based on the provided algorithm string
            let algorithm = match remote_fingerprint_algorithm.to_lowercase().as_str() {
                "sha-1" => "SHA-1",
                "sha-256" => "SHA-256",
                "sha-384" => "SHA-384",
                "sha-512" => "SHA-512",
                _ => {
                    return Err(SecurityError::HandshakeVerification(
                        format!("Unsupported fingerprint algorithm: {}", remote_fingerprint_algorithm)
                    ));
                }
            };
            
            // Generate the fingerprint using the specified algorithm
            match cert_copy.fingerprint(algorithm) {
                Ok(actual_fingerprint) => {
                    // Normalize both fingerprints for comparison (remove colons, convert to lowercase)
                    let expected = remote_fingerprint.replace(":", "").to_lowercase();
                    let actual = actual_fingerprint.replace(":", "").to_lowercase();
                    
                    // Compare the fingerprints
                    let matches = expected == actual;
                    
                    if matches {
                        debug!("Fingerprint verification successful");
                    } else {
                        warn!("Fingerprint verification failed! Expected: {}, Actual: {}", 
                              remote_fingerprint, actual_fingerprint);
                    }
                    
                    Ok(matches)
                },
                Err(e) => Err(SecurityError::HandshakeVerification(
                    format!("Failed to generate fingerprint for verification: {}", e)
                )),
            }
        } else {
            Err(SecurityError::HandshakeVerification("No remote certificate available for verification".to_string()))
        }
    } else {
        Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
    }
}

/// Get the fingerprint algorithm used
pub fn get_fingerprint_algorithm() -> String {
    // We use SHA-256 as the default fingerprint algorithm
    "sha-256".to_string()
} 