//! Media security module
//!
//! This module provides implementations for securing media streams
//! through SRTP (Secure RTP) and DTLS key exchange.
//!
//! This module integrates with the rtp-core crate's implementations.

// Re-export SRTP implementation
pub mod srtp;

// Re-export DTLS implementation
pub mod dtls;

// Re-export key types
pub use srtp::{SrtpSession, SrtpConfig, SrtpKeys};
pub use dtls::{DtlsConnection, DtlsConfig, DtlsEvent, DtlsRole, TransportConn};

// Re-export types from rtp-core for direct use
pub use rvoip_rtp_core::dtls::{DtlsVersion};
pub use rvoip_rtp_core::srtp::{
    SrtpContext, SrtpEncryptionAlgorithm, SrtpAuthenticationAlgorithm,
    SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32
}; 