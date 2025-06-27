//! MIKEY payload definitions
//!
//! This module defines the payload types used in MIKEY messages as specified in RFC 3830.

/// MIKEY payload types as defined in RFC 3830
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PayloadType {
    /// Last payload (used to mark the end of payloads)
    Last = 0,
    /// Key data transport payload
    KeyData = 1,
    /// Timestamp payload
    Timestamp = 2,
    /// Random number (RAND) payload
    Rand = 3,
    /// Security policy payload
    SecurityPolicy = 4,
    /// Key validation data payload (for PSK mode)
    KeyValidationData = 5,
    /// General extensions payload
    GeneralExtension = 6,
    /// Certificate payload (for PKE mode)
    Certificate = 7,
    /// Encrypted payload (for PKE mode)
    Encrypted = 8,
    /// MAC (Message Authentication Code) payload
    Mac = 9,
    /// Signature payload (for PKE mode)
    Signature = 10,
    /// Public Key payload (for PKE mode)
    PublicKey = 11,
    /// Unknown payload type
    Unknown = 255,
}

/// MIKEY Common Header (HDR) payload
#[derive(Debug, Clone)]
pub struct CommonHeader {
    /// Protocol version (3 bits)
    pub version: u8,
    /// Message type (3 bits)
    pub data_type: u8,
    /// Next payload (8 bits)
    pub next_payload: u8,
    /// Verification flag (1 bit)
    pub v_flag: bool,
    /// PRF function (7 bits)
    pub prf_func: u8,
    /// CSP ID map type (8 bits)
    pub csp_id: u16,
    /// CS counter (8 bits)
    pub cs_count: u8,
    /// CS ID map type (8 bits)
    pub cs_id_map_type: u8,
}

/// MIKEY Key Data payload
#[derive(Debug, Clone)]
pub struct KeyDataPayload {
    /// Key type (1 byte)
    pub key_type: u8,
    /// Key data
    pub key_data: Vec<u8>,
    /// Salt data (optional)
    pub salt_data: Option<Vec<u8>>,
    /// Key validity data (optional)
    pub kv_data: Option<Vec<u8>>,
}

/// MIKEY Security Policy payload
#[derive(Debug, Clone)]
pub struct SecurityPolicyPayload {
    /// Policy number (1 byte)
    pub policy_no: u8,
    /// Policy type (1 byte)
    pub policy_type: u8,
    /// Policy parameters
    pub policy_param: Vec<u8>,
}

/// MIKEY Key Validation Data payload
#[derive(Debug, Clone)]
pub struct KeyValidationData {
    /// Validation data (MAC or signature)
    pub validation_data: Vec<u8>,
}

/// MIKEY General Extension payload
#[derive(Debug, Clone)]
pub struct GeneralExtensionPayload {
    /// Extension type (1 byte)
    pub ext_type: u8,
    /// Extension data
    pub ext_data: Vec<u8>,
}

/// MIKEY Certificate payload (PKE mode)
#[derive(Debug, Clone)]
pub struct CertificatePayload {
    /// Certificate type (X.509, PGP, etc.)
    pub cert_type: CertificateType,
    /// Certificate data (DER-encoded for X.509)
    pub cert_data: Vec<u8>,
    /// Certificate chain (optional additional certificates)
    pub cert_chain: Vec<Vec<u8>>,
}

/// Certificate types supported in MIKEY
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CertificateType {
    /// X.509 certificate (DER encoded)
    X509 = 0,
    /// X.509 certificate chain
    X509Chain = 1,
    /// PGP certificate
    Pgp = 2,
    /// Reserved for future use
    Reserved = 255,
}

/// MIKEY Signature payload (PKE mode)
#[derive(Debug, Clone)]
pub struct SignaturePayload {
    /// Signature algorithm
    pub sig_algorithm: SignatureAlgorithm,
    /// Signature data
    pub signature: Vec<u8>,
}

/// Signature algorithms supported in MIKEY
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SignatureAlgorithm {
    /// RSA with SHA-256 (PKCS#1 v1.5)
    RsaSha256 = 0,
    /// RSA with SHA-512 (PKCS#1 v1.5)
    RsaSha512 = 1,
    /// ECDSA with SHA-256
    EcdsaSha256 = 2,
    /// ECDSA with SHA-512
    EcdsaSha512 = 3,
    /// RSA-PSS with SHA-256
    RsaPssSha256 = 4,
    /// RSA-PSS with SHA-512
    RsaPssSha512 = 5,
}

/// MIKEY Encrypted payload (PKE mode)
#[derive(Debug, Clone)]
pub struct EncryptedPayload {
    /// Encryption algorithm used
    pub enc_algorithm: EncryptionAlgorithm,
    /// Encrypted data
    pub encrypted_data: Vec<u8>,
    /// Initialization vector (if needed)
    pub iv: Option<Vec<u8>>,
}

/// Encryption algorithms for MIKEY PKE
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EncryptionAlgorithm {
    /// RSA PKCS#1 v1.5
    RsaPkcs1 = 0,
    /// RSA OAEP with SHA-256
    RsaOaepSha256 = 1,
    /// ECIES (Elliptic Curve Integrated Encryption Scheme)
    Ecies = 2,
}

/// MIKEY Public Key payload (PKE mode)
#[derive(Debug, Clone)]
pub struct PublicKeyPayload {
    /// Public key algorithm
    pub key_algorithm: PublicKeyAlgorithm,
    /// Public key data
    pub key_data: Vec<u8>,
    /// Key parameters (curve OID for ECC, modulus size for RSA)
    pub key_params: Option<Vec<u8>>,
}

/// Public key algorithms supported in MIKEY
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PublicKeyAlgorithm {
    /// RSA public key
    Rsa = 0,
    /// ECDSA public key (P-256)
    EcdsaP256 = 1,
    /// ECDSA public key (P-384)
    EcdsaP384 = 2,
    /// ECDSA public key (P-521)
    EcdsaP521 = 3,
    /// Ed25519 public key
    Ed25519 = 4,
}

impl CertificateType {
    /// Check if this certificate type requires a chain
    pub fn supports_chain(&self) -> bool {
        matches!(self, CertificateType::X509Chain)
    }
}

impl SignatureAlgorithm {
    /// Get the hash algorithm used by this signature algorithm
    pub fn hash_algorithm(&self) -> &'static str {
        match self {
            SignatureAlgorithm::RsaSha256 | 
            SignatureAlgorithm::EcdsaSha256 | 
            SignatureAlgorithm::RsaPssSha256 => "SHA-256",
            SignatureAlgorithm::RsaSha512 | 
            SignatureAlgorithm::EcdsaSha512 | 
            SignatureAlgorithm::RsaPssSha512 => "SHA-512",
        }
    }
    
    /// Check if this is an RSA-based algorithm
    pub fn is_rsa(&self) -> bool {
        matches!(self, 
            SignatureAlgorithm::RsaSha256 | 
            SignatureAlgorithm::RsaSha512 |
            SignatureAlgorithm::RsaPssSha256 |
            SignatureAlgorithm::RsaPssSha512
        )
    }
    
    /// Check if this is an ECDSA-based algorithm
    pub fn is_ecdsa(&self) -> bool {
        matches!(self, 
            SignatureAlgorithm::EcdsaSha256 | 
            SignatureAlgorithm::EcdsaSha512
        )
    }
}

impl EncryptionAlgorithm {
    /// Check if this algorithm requires an IV
    pub fn requires_iv(&self) -> bool {
        matches!(self, EncryptionAlgorithm::Ecies)
    }
    
    /// Get the expected key size for this algorithm (in bytes)
    pub fn key_size(&self) -> Option<usize> {
        match self {
            EncryptionAlgorithm::RsaPkcs1 => None, // Variable based on RSA key size
            EncryptionAlgorithm::RsaOaepSha256 => None, // Variable based on RSA key size
            EncryptionAlgorithm::Ecies => Some(32), // P-256 key size
        }
    }
}

impl PublicKeyAlgorithm {
    /// Get the expected key size for this algorithm (in bytes)
    pub fn key_size(&self) -> usize {
        match self {
            PublicKeyAlgorithm::Rsa => 256, // 2048-bit RSA (variable)
            PublicKeyAlgorithm::EcdsaP256 => 64, // 32 bytes x + 32 bytes y
            PublicKeyAlgorithm::EcdsaP384 => 96, // 48 bytes x + 48 bytes y
            PublicKeyAlgorithm::EcdsaP521 => 132, // 66 bytes x + 66 bytes y
            PublicKeyAlgorithm::Ed25519 => 32, // 32 bytes
        }
    }
    
    /// Get the curve name for ECC algorithms
    pub fn curve_name(&self) -> Option<&'static str> {
        match self {
            PublicKeyAlgorithm::EcdsaP256 => Some("P-256"),
            PublicKeyAlgorithm::EcdsaP384 => Some("P-384"),
            PublicKeyAlgorithm::EcdsaP521 => Some("P-521"),
            PublicKeyAlgorithm::Ed25519 => Some("Ed25519"),
            PublicKeyAlgorithm::Rsa => None,
        }
    }
} 