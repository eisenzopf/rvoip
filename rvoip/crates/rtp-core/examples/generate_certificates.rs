/// Certificate Generator for Secure Media Testing
///
/// This example generates self-signed certificates and private keys for testing
/// the secure media streaming example with DTLS-SRTP.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use rcgen::{Certificate, CertificateParams, PKCS_ECDSA_P256_SHA256};
use rvoip_rtp_core::dtls::crypto::verify::generate_self_signed_certificate;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Certificate Generator for Secure Media Testing");
    println!("=============================================\n");
    
    // Generate server certificate using rcgen directly
    println!("Generating server certificate...");
    let mut params = CertificateParams::new(vec!["localhost".to_string()]);
    params.alg = &PKCS_ECDSA_P256_SHA256;
    params.distinguished_name.push(rcgen::DnType::OrganizationName, "RVOIP Test Server");
    params.distinguished_name.push(rcgen::DnType::CommonName, "server.rvoip.test");
    
    let server_cert = Certificate::from_params(params)?;
    
    // Save server certificate and private key
    let server_cert_path = "server-cert.pem";
    let server_key_path = "server-key.pem";
    
    // Save PEM files
    let mut cert_file = File::create(server_cert_path)?;
    cert_file.write_all(server_cert.serialize_pem()?.as_bytes())?;
    
    let mut key_file = File::create(server_key_path)?;
    key_file.write_all(server_cert.serialize_private_key_pem().as_bytes())?;
    
    println!("Saved server certificate to {} and private key to {}", server_cert_path, server_key_path);
    
    // Generate client certificate using rcgen
    println!("Generating client certificate...");
    let mut params = CertificateParams::new(vec!["localhost".to_string()]);
    params.alg = &PKCS_ECDSA_P256_SHA256;
    params.distinguished_name.push(rcgen::DnType::OrganizationName, "RVOIP Test Client");
    params.distinguished_name.push(rcgen::DnType::CommonName, "client.rvoip.test");
    
    let client_cert = Certificate::from_params(params)?;
    
    // Save client certificate and private key
    let client_cert_path = "client-cert.pem";
    let client_key_path = "client-key.pem";
    
    // Save PEM files
    let mut cert_file = File::create(client_cert_path)?;
    cert_file.write_all(client_cert.serialize_pem()?.as_bytes())?;
    
    let mut key_file = File::create(client_key_path)?;
    key_file.write_all(client_cert.serialize_private_key_pem().as_bytes())?;
    
    println!("Saved client certificate to {} and private key to {}", client_cert_path, client_key_path);
    
    // Display fingerprints for verification
    // First, let's generate certificates using the built-in utility to get fingerprints
    let server_internal_cert = generate_self_signed_certificate()?;
    let client_internal_cert = generate_self_signed_certificate()?;
    
    let mut server_internal_cert = server_internal_cert;
    let mut client_internal_cert = client_internal_cert;
    
    let server_fingerprint = server_internal_cert.fingerprint("sha-256")?;
    let client_fingerprint = client_internal_cert.fingerprint("sha-256")?;
    
    println!("\nCertificate details:");
    println!("===================");
    println!("Server certificate fingerprint (SHA-256): {}", server_fingerprint);
    println!("Client certificate fingerprint (SHA-256): {}", client_fingerprint);
    
    println!("\nFiles created:");
    println!("server-cert.pem - Server certificate (public key)");
    println!("server-key.pem - Server private key");
    println!("client-cert.pem - Client certificate (public key)");
    println!("client-key.pem - Client private key");
    
    println!("\nNote: The fingerprints shown are for randomly generated internal certificates,");
    println!("not for the certificates saved to files. These are for illustration only.");
    println!("\nYou can now use these files with the secure_media_streaming example.");
    Ok(())
} 