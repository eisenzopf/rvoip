//! Security mechanisms for SIP/RTP
//!
//! This module provides security mechanisms used in SIP and RTP communications:
//! 
//! - MIKEY (Multimedia Internet KEYing): Protocol for key management 
//! - SDES (Security DEScriptions): Method for exchanging SRTP keys via SDP
//! - ZRTP (Z Real-time Transport Protocol): Key agreement protocol for SRTP

pub mod mikey;
pub mod sdes;
pub mod zrtp;

use crate::srtp::crypto::SrtpCryptoKey;
use crate::srtp::SrtpCryptoSuite;
use crate::Error;

/// Trait implemented by all security key exchange methods
pub trait SecurityKeyExchange {
    /// Initialize the key exchange process
    fn init(&mut self) -> Result<(), Error>;
    
    /// Process incoming message from peer
    fn process_message(&mut self, message: &[u8]) -> Result<Option<Vec<u8>>, Error>;
    
    /// Get the negotiated SRTP crypto key if available
    fn get_srtp_key(&self) -> Option<SrtpCryptoKey>;
    
    /// Get the negotiated SRTP crypto suite if available
    fn get_srtp_suite(&self) -> Option<SrtpCryptoSuite>;
    
    /// Check if the key exchange is complete
    fn is_complete(&self) -> bool;
} 